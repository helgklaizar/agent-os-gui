//! `fake-agentd` SQLite (db.md section 2): single writer, reads on separate connections, migrations.
//!
//! One writer: the `<DB>.lock` file holds an exclusive lock, inside the process writes go through one `Mutex`.

mod migrate;
mod snapshot;
mod storage;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

pub use migrate::{FreeSpace, MIGRATIONS, Migration, migrate, migrate_with_space};

use crate::disk::{DiskEncryption, DiskWarning};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("DB is newer than the code: user_version {db}, known {known}")]
    UnknownVersion { db: i64, known: i64 },
    #[error("schema_migrations has version {version} without a migration file")]
    MissingMigrationFile { version: i64 },
    #[error("version {version}: the name in schema_migrations does not match the file")]
    MigrationMismatch { version: i64 },
    #[error("user_version {user_version} is not equal to max(schema_migrations) {schema_max}")]
    VersionMismatch { user_version: i64, schema_max: i64 },
    #[error("foreign_key_check after migration {version} is non-empty")]
    ForeignKeyViolation { version: i64 },
    #[error("not enough space for the migration: free {free}, need {need}")]
    LowDisk { free: u64, need: u64 },
    #[error("the migration set is unordered or has duplicates")]
    InvalidMigrationSet,
    #[error("the DB writer is already held by another process or connection")]
    WriterBusy,
    #[error("pragma {name}: got {got}")]
    Pragma { name: &'static str, got: String },
}

/// Result of the storage check at open.
#[derive(Debug, Clone)]
pub struct StorageReport {
    pub encryption: DiskEncryption,
    pub warning: Option<DiskWarning>,
    pub perms_tightened: bool,
}

/// DB with a single writer.
pub struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    report: StorageReport,
    _lock: File,
}

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

fn expect_pragma(name: &'static str, got: String, want: &str) -> Result<(), DbError> {
    if got.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err(DbError::Pragma { name, got })
    }
}

/// The migration does not start (db.md section 6: DB newer than the code, low space): an `error` event is written next to the DB
/// (`<DB>.events.jsonl`), the DB itself is untouched. The event text has no paths or data: only the type and code.
fn report_error(path: &Path, code: &str) -> Result<(), DbError> {
    use std::io::Write;
    let mut name = path.as_os_str().to_owned();
    name.push(".events.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(PathBuf::from(name))?;
    writeln!(file, r#"{{"type":"error","code":"{code}"}}"#)?;
    Ok(())
}

impl Db {
    /// Opens the DB at `path` with migrations from `fake-agentd/migrations/`.
    pub fn open(path: &Path) -> Result<Db, DbError> {
        Db::open_with(path, MIGRATIONS)
    }

    /// Same with a given migration set (runner tests).
    pub fn open_with(path: &Path, migrations: &[Migration]) -> Result<Db, DbError> {
        Db::open_with_space(path, migrations, &agentd_sys::free_bytes)
    }

    /// Same with a given free space source (space check before a migration).
    pub fn open_with_space(
        path: &Path,
        migrations: &[Migration],
        free_space: FreeSpace<'_>,
    ) -> Result<Db, DbError> {
        let lock = storage::prepare(path)?;
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        expect_pragma("journal_mode", mode, "wal")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        expect_pragma("foreign_keys", fk.to_string(), "1")?;
        if let Err(e) = migrate_with_space(&mut conn, migrations, free_space) {
            match e {
                DbError::UnknownVersion { .. } => report_error(path, "db_version_unsupported")?,
                DbError::LowDisk { .. } => report_error(path, "low_disk")?,
                _ => {}
            }
            return Err(e);
        }
        let report = storage::finish(path)?;
        Ok(Db {
            path: path.to_path_buf(),
            writer: Mutex::new(conn),
            report,
            _lock: lock,
        })
    }

    /// The only write point: the closure runs under a mutex on the single write connection.
    pub fn write<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, DbError> {
        let mut guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        Ok(f(&mut guard)?)
    }

    /// New read-only connection (`SQLITE_OPEN_READ_ONLY` and `query_only`).
    pub fn reader(&self) -> Result<Connection, DbError> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(conn)
    }

    /// Result of the storage check at open: disk encryption and permissions.
    pub fn storage_report(&self) -> &StorageReport {
        &self.report
    }
}
