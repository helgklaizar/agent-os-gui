//! DB integrity (S10-DB-05): `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, `PRAGMA user_version` agreement with
//! `schema_migrations`. Used after restore, after archive verification and after retention. The `error` event for
//! command-line processes is written by `record_event`.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use crate::{DbError, MIGRATIONS};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Integrity {
    /// First row of `PRAGMA integrity_check` (`ok` or a description of the corruption).
    pub integrity: String,
    /// Number of rows in `PRAGMA foreign_key_check`.
    pub fk_violations: u64,
    pub user_version: i64,
    /// `max(version)` from `schema_migrations`; 0 means no table or no rows.
    pub applied_max: i64,
}

impl Integrity {
    /// Schema version this code knows.
    #[must_use]
    pub fn known_version() -> i64 {
        MIGRATIONS.last().map_or(0, |m| m.version)
    }

    /// Everything that prevents considering the DB fit; empty means fit. Data values do not enter the texts.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.integrity != "ok" {
            out.push(format!("integrity_check: {}", self.integrity));
        }
        if self.fk_violations > 0 {
            out.push(format!(
                "foreign_key_check: {} violations",
                self.fk_violations
            ));
        }
        if self.user_version != self.applied_max {
            out.push(format!(
                "user_version {} does not match schema_migrations {}",
                self.user_version, self.applied_max
            ));
        }
        if self.user_version > Self::known_version() {
            out.push(format!(
                "DB is newer than the code: version {} while known is {}",
                self.user_version,
                Self::known_version()
            ));
        }
        out
    }
}

/// Checks on an open connection (read only).
pub fn check_conn(conn: &Connection) -> rusqlite::Result<Integrity> {
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let fk_violations = conn
        .prepare("PRAGMA foreign_key_check")?
        .query_map([], |_| Ok(()))?
        .count() as u64;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |r| r.get(0),
    )?;
    let applied_max = if has == 0 {
        0
    } else {
        conn.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?
    };
    Ok(Integrity {
        integrity,
        fk_violations,
        user_version,
        applied_max,
    })
}

/// The same for a DB file: opened read-only, the file does not change (no `-wal` is created for a snapshot without a journal).
pub fn check_file(path: &Path) -> Result<Integrity, DbError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(check_conn(&conn)?)
}

/// An event into the feed from a command-line process (backup, archive verification): a short write to an existing DB, waiting
/// for the lock up to 5 s. Does not create a new DB; level is `error`, `warn`, `ok` or `info`.
pub fn record_event(db: &Path, level: &str, text: &str) -> Result<(), DbError> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    conn.execute(
        "INSERT INTO events(kind, sub, level, text, source, unread, ts) VALUES ('msg', 'sys', ?1, ?2, 'agent-backup', 1, ?3)",
        rusqlite::params![level, text, ts],
    )?;
    Ok(())
}

/// Public `age` keys from `settings` (`backup.recipient.primary`, `backup.recipient.recovery`, docs/adr/0007-distribution-and-backup-keys.md item 2).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackupRecipients {
    pub primary: Option<String>,
    pub recovery: Option<String>,
}

pub fn backup_recipients(db: &Path) -> Result<BackupRecipients, DbError> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let get = |key: &str| -> Result<Option<String>, DbError> {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(raw.and_then(|j| serde_json::from_str::<String>(&j).ok()))
    };
    Ok(BackupRecipients {
        primary: get("backup.recipient.primary")?,
        recovery: get("backup.recipient.recovery")?,
    })
}
