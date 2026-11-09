//! Migration runner (docs/reference/db.md section 6, docs/adr/0002-db-single-writer.md).
//!
//! Order of one migration: `foreign_keys = OFF` outside a transaction (if it recreates tables) → `BEGIN` → file → row in
//! `schema_migrations` → `foreign_key_check` (non-empty: rollback) → `COMMIT` → `foreign_keys = ON` → `PRAGMA user_version = N`.
//! `user_version` and `schema_migrations` are written only by the runner (and only this file). There are no "down" migrations.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::DbError;
use crate::snapshot;

/// One migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
    /// Changes or moves data: a `VACUUM INTO` snapshot is taken before it.
    pub mutates_data: bool,
    /// Recreates tables: `foreign_keys = OFF` outside a transaction for the duration of the migration.
    pub recreates_tables: bool,
}

/// Runner events: progress for `/health` and errors (without user data).
#[derive(Debug, Clone, PartialEq)]
pub enum RunnerEvent {
    Started { from: i64, to: i64 },
    Applied { version: i64, name: String },
    Finished { version: i64 },
    Error { code: &'static str },
}

pub type FreeSpaceFn = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;
pub type EventFn = Arc<dyn Fn(&RunnerEvent) + Send + Sync>;
pub type StampFn = Arc<dyn Fn() -> String + Send + Sync>;

pub struct MigrateOptions {
    /// Snapshot directory (`~/.agentos/migration-backups/`); `None` means no snapshots are taken.
    pub backup_dir: Option<PathBuf>,
    /// Source of free space (replaced in tests).
    pub free_space: FreeSpaceFn,
    pub on_event: Option<EventFn>,
    pub date_stamp: StampFn,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        MigrateOptions {
            backup_dir: None,
            free_space: Arc::new(agentd_sys::free_bytes),
            on_event: None,
            date_stamp: Arc::new(snapshot::today_stamp),
        }
    }
}

impl MigrateOptions {
    fn emit(&self, e: &RunnerEvent) {
        if let Some(f) = &self.on_event {
            f(e);
        }
    }

    /// `error` event and a log entry; the error is returned to the caller as is.
    fn fail<T>(&self, code: &'static str, err: DbError) -> Result<T, DbError> {
        tracing::error!(event = "error", code, "migrations stopped");
        self.emit(&RunnerEvent::Error { code });
        Err(err)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationOutcome {
    pub applied: Vec<i64>,
    pub snapshots: Vec<PathBuf>,
}

/// Current schema version (read only).
pub fn user_version(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn set_user_version(conn: &Connection, version: i64) -> Result<(), DbError> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}

/// Applied versions; a DB without a `schema_migrations` table gives an empty list.
fn applied(conn: &Connection) -> Result<Vec<(i64, String)>, DbError> {
    let has: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |r| r.get(0),
    )?;
    if has == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT version, name FROM schema_migrations ORDER BY version")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn apply_one(conn: &mut Connection, m: &Migration) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    tx.execute_batch(m.sql)?;
    tx.execute(
        "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![m.version, m.name, now_unix()],
    )?;
    let violations: i64 =
        tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if violations > 0 {
        return Err(DbError::ForeignKeyViolation { version: m.version });
    }
    tx.commit()?;
    Ok(())
}

fn check_set(migrations: &[Migration]) -> Result<(), DbError> {
    let bad = migrations
        .iter()
        .enumerate()
        .any(|(i, m)| m.version <= 0 || (i > 0 && m.version <= migrations[i - 1].version));
    if bad {
        Err(DbError::InvalidMigrationSet)
    } else {
        Ok(())
    }
}

/// Compare what is applied with the set: names, files, `user_version` does not run ahead of `schema_migrations`.
fn check_applied(migrations: &[Migration], done: &[(i64, String)], uv: i64) -> Result<(), DbError> {
    for (i, (version, name)) in done.iter().enumerate() {
        let Some(file) = migrations.iter().find(|m| m.version == *version) else {
            return Err(DbError::MissingMigrationFile { version: *version });
        };
        if file.name != name || migrations.get(i).map(|m| m.version) != Some(*version) {
            return Err(DbError::MigrationMismatch { version: *version });
        }
    }
    let schema_max = done.last().map_or(0, |(v, _)| *v);
    if uv > schema_max {
        return Err(DbError::VersionMismatch {
            user_version: uv,
            schema_max,
        });
    }
    Ok(())
}

/// Free space of at least twice the DB size (docs/reference/db.md section 6). No check for an in-memory DB or an empty file.
fn check_disk(conn: &Connection, opts: &MigrateOptions) -> Result<(), DbError> {
    let Some(path) = conn.path().filter(|p| !p.is_empty()).map(PathBuf::from) else {
        return Ok(());
    };
    let size = std::fs::metadata(&path)?.len();
    let need = size.saturating_mul(2);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let free = (opts.free_space)(dir)?;
    if free < need {
        return Err(DbError::LowDisk { free, need });
    }
    Ok(())
}

/// Brings the schema to the latest version in the set. A DB newer than the code, an inconsistent state, or low space: refusal without a write.
pub fn migrate(
    conn: &mut Connection,
    migrations: &[Migration],
    opts: &MigrateOptions,
) -> Result<MigrationOutcome, DbError> {
    check_set(migrations)?;
    let known = migrations.last().map_or(0, |m| m.version);
    let uv = user_version(conn)?;
    if uv > known {
        return opts.fail(
            "db_newer_than_code",
            DbError::NewerThanCode { db: uv, known },
        );
    }
    let done = applied(conn)?;
    if let Err(e) = check_applied(migrations, &done, uv) {
        return opts.fail("schema_inconsistent", e);
    }
    let schema_max = done.last().map_or(0, |(v, _)| *v);
    let mut out = MigrationOutcome::default();
    let pending = &migrations[done.len()..];
    if pending.is_empty() {
        if uv < schema_max {
            // interrupted between COMMIT and the user_version write: catch up
            set_user_version(conn, schema_max)?;
        }
        return Ok(out);
    }
    if let Err(e) = check_disk(conn, opts) {
        return opts.fail("low_disk", e);
    }
    opts.emit(&RunnerEvent::Started {
        from: uv,
        to: known,
    });
    for m in pending {
        if let Err(e) = run_one(conn, m, uv, opts, &mut out) {
            return opts.fail("migration_failed", e);
        }
        opts.emit(&RunnerEvent::Applied {
            version: m.version,
            name: m.name.to_string(),
        });
    }
    opts.emit(&RunnerEvent::Finished { version: known });
    Ok(out)
}

fn run_one(
    conn: &mut Connection,
    m: &Migration,
    initial_version: i64,
    opts: &MigrateOptions,
    out: &mut MigrationOutcome,
) -> Result<(), DbError> {
    if m.mutates_data
        && initial_version > 0
        && let Some(dir) = &opts.backup_dir
    {
        out.snapshots
            .push(snapshot::take(conn, dir, m.version, &(opts.date_stamp)())?);
    }
    if m.recreates_tables {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let result = apply_one(conn, m);
    if m.recreates_tables {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    result?;
    set_user_version(conn, m.version)?;
    out.applied.push(m.version);
    Ok(())
}

/// Result of `migrate --check`: what is in the DB and what the code knows. Creates and changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationCheck {
    pub db_version: i64,
    pub known_version: i64,
}

impl MigrationCheck {
    #[must_use]
    pub fn pending(&self) -> bool {
        self.db_version < self.known_version
    }

    #[must_use]
    pub fn newer_than_code(&self) -> bool {
        self.db_version > self.known_version
    }
}

/// Reads the schema version read-only; no DB file means version 0 (the file is not created).
pub fn check_migrations(path: &Path, migrations: &[Migration]) -> Result<MigrationCheck, DbError> {
    let known_version = migrations.last().map_or(0, |m| m.version);
    if !path.exists() {
        return Ok(MigrationCheck {
            db_version: 0,
            known_version,
        });
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(MigrationCheck {
        db_version: user_version(&conn)?,
        known_version,
    })
}
