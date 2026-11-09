//! Migration runner (db.md section 6).
//!
//! Order of one migration: `foreign_keys = OFF` outside a transaction -> `BEGIN` -> file -> row in `schema_migrations` ->
//! `foreign_key_check` (non-empty - rollback) -> `COMMIT` -> `foreign_keys = ON` -> `PRAGMA user_version = N`.
//! `user_version` and `schema_migrations` are written only by the runner. There are no "down" migrations.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::{DbError, snapshot};

/// One migration: version, file name without the number and the SQL text.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
    /// Changes or moves data: a snapshot is taken before it (db.md section 6). Same values as in
    /// the core catalog (the single source for the real core).
    pub mutates_data: bool,
}

/// Known migrations in ascending version order; 0001, 0002, 0006, 0007 and 0008 come from `agentd/migrations/` (the single source), the rest are in `fake-agentd/migrations/`.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../../agentd/migrations/0001_init.sql"),
        mutates_data: false,
    },
    Migration {
        version: 2,
        name: "bar",
        sql: include_str!("../../../agentd/migrations/0002_bar.sql"),
        mutates_data: false,
    },
    Migration {
        version: 3,
        name: "sessions",
        sql: include_str!("../../migrations/0003_sessions.sql"),
        mutates_data: false,
    },
    Migration {
        version: 4,
        name: "chat",
        sql: include_str!("../../migrations/0004_chat.sql"),
        mutates_data: false,
    },
    Migration {
        version: 5,
        name: "events_gateway",
        sql: include_str!("../../migrations/0005_events_gateway.sql"),
        mutates_data: false,
    },
    Migration {
        version: 6,
        name: "tasks",
        sql: include_str!("../../../agentd/migrations/0006_tasks.sql"),
        mutates_data: false,
    },
    Migration {
        version: 7,
        name: "modules_tunnels",
        sql: include_str!("../../../agentd/migrations/0007_modules_tunnels.sql"),
        mutates_data: false,
    },
    Migration {
        version: 8,
        name: "memory_facts",
        sql: include_str!("../../../agentd/migrations/0008_memory_facts.sql"),
        mutates_data: false,
    },
    Migration {
        version: 9,
        name: "web_token",
        sql: include_str!("../../../agentd/migrations/0009_web_token.sql"),
        // recreating `api_token`: data is moved, as in the core catalog (`mutates_data: true`)
        mutates_data: true,
    },
    Migration {
        version: 10,
        name: "permissions_mcp",
        sql: include_str!("../../../agentd/migrations/0010_permissions_mcp.sql"),
        mutates_data: false,
    },
];

fn user_version(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Applied versions; a DB without the `schema_migrations` table gives an empty list.
fn applied(conn: &Connection) -> Result<Vec<(i64, String)>, DbError> {
    let has_table: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |r| r.get(0),
    )?;
    if has_table == 0 {
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

/// Free space at least twice the DB size (db.md section 6). No check for an in-memory DB.
fn check_disk(conn: &Connection, free_space: FreeSpace<'_>) -> Result<(), DbError> {
    let Some(path) = conn.path().filter(|p| !p.is_empty()).map(Path::new) else {
        return Ok(());
    };
    let need = std::fs::metadata(path)?.len().saturating_mul(2);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let free = free_space(dir)?;
    if free < need {
        return Err(DbError::LowDisk { free, need });
    }
    Ok(())
}

/// Free space source by DB directory (replaced in tests).
pub type FreeSpace<'a> = &'a dyn Fn(&Path) -> std::io::Result<u64>;

/// Brings the schema to the latest known version. Writes nothing if the DB is newer than the code or the state is inconsistent.
pub fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<(), DbError> {
    migrate_with_space(conn, migrations, &agentd_sys::free_bytes)
}

/// Same with a given free space source (db.md section 6).
pub fn migrate_with_space(
    conn: &mut Connection,
    migrations: &[Migration],
    free_space: FreeSpace<'_>,
) -> Result<(), DbError> {
    if migrations
        .iter()
        .enumerate()
        .any(|(i, m)| m.version <= 0 || (i > 0 && m.version <= migrations[i - 1].version))
    {
        return Err(DbError::InvalidMigrationSet);
    }
    let known = migrations.last().map_or(0, |m| m.version);
    let uv = user_version(conn)?;
    if uv > known {
        return Err(DbError::UnknownVersion { db: uv, known });
    }

    let done = applied(conn)?;
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

    if done.len() < migrations.len() {
        check_disk(conn, free_space)?;
    }
    let mut current = uv;
    for m in &migrations[done.len()..] {
        if uv > 0 && m.mutates_data {
            snapshot::take(conn, m.version)?;
        }
        conn.pragma_update(None, "foreign_keys", false)?;
        let result = apply_one(conn, m);
        conn.pragma_update(None, "foreign_keys", true)?;
        result?;
        conn.pragma_update(None, "user_version", m.version)?;
        current = m.version;
    }
    if current < schema_max {
        // a break between COMMIT and the user_version write: catch up
        conn.pragma_update(None, "user_version", schema_max)?;
    }
    Ok(())
}
