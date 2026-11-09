//! Settings pages in the `settings` table (db.md section 3: `key`, `value_json`, `rev`, `updated_at`).
//!
//! `rev` grows on every write; a write with a stale `rev` fails (`If-Match`).

use std::sync::Arc;

use rusqlite::params;

use crate::db::Db;
use crate::store::{StoreError, db_err, map_db};

const THEME_KEY: &str = "theme";

/// Result of a theme write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeWrite {
    /// Written, new `rev`.
    Saved(i64),
    /// The `rev` in the request did not match the current one.
    Stale,
}

pub struct SettingsStore {
    db: Arc<Db>,
}

impl SettingsStore {
    /// Creates `theme = "auto"`, `rev = 1` if the row does not exist yet (leaves an existing row alone).
    pub fn new(db: Arc<Db>, now: i64) -> Result<SettingsStore, StoreError> {
        db.write(|c| {
            c.execute(
                "INSERT OR IGNORE INTO settings (key, value_json, rev, updated_at) VALUES (?1, '\"auto\"', 1, ?2)",
                params![THEME_KEY, now],
            )
        })
        .map_err(map_db)?;
        Ok(SettingsStore { db })
    }

    pub fn theme(&self) -> Result<(String, i64), StoreError> {
        let conn = self.db.reader().map_err(db_err)?;
        let (json, rev): (String, i64) = conn
            .query_row(
                "SELECT value_json, rev FROM settings WHERE key = ?1",
                params![THEME_KEY],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_err)?;
        let theme = serde_json::from_str::<String>(&json).map_err(db_err)?;
        Ok((theme, rev))
    }

    pub fn set_theme(
        &self,
        want_rev: i64,
        theme: &str,
        now: i64,
    ) -> Result<ThemeWrite, StoreError> {
        let json = serde_json::to_string(theme).map_err(db_err)?;
        self.db
            .write(|c| {
                let tx = c.transaction()?;
                let n = tx.execute(
                    "UPDATE settings SET value_json = ?1, rev = rev + 1, updated_at = ?2 WHERE key = ?3 AND rev = ?4",
                    params![json, now, THEME_KEY, want_rev],
                )?;
                if n == 0 {
                    return Ok(ThemeWrite::Stale);
                }
                let rev: i64 = tx.query_row(
                    "SELECT rev FROM settings WHERE key = ?1",
                    params![THEME_KEY],
                    |r| r.get(0),
                )?;
                tx.commit()?;
                Ok(ThemeWrite::Saved(rev))
            })
            .map_err(map_db)
    }
}
