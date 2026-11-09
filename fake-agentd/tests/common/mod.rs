//! Shared DB test helpers. Tests go through the live `Db::open` and `migrate`.
#![allow(dead_code)]

use std::path::PathBuf;

use fake_agentd::Db;
use rusqlite::Connection;
use tempfile::TempDir;

pub fn temp_db_path() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sessions.db");
    (dir, path)
}

pub fn open(path: &std::path::Path) -> Db {
    Db::open(path).expect("open the DB")
}

/// Versions from `schema_migrations`; no table - an empty list (the test fails on an assertion, not on a query).
pub fn applied_versions(conn: &Connection) -> Vec<i64> {
    let Ok(mut stmt) = conn.prepare("SELECT version FROM schema_migrations ORDER BY version")
    else {
        return Vec::new();
    };
    stmt.query_map([], |r| r.get(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

pub fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version")
}

pub fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        [kind, name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n == 1)
    .unwrap_or(false)
}

pub fn pragma_text(conn: &Connection, name: &str) -> String {
    conn.query_row(&format!("PRAGMA {name}"), [], |r| {
        r.get::<_, rusqlite::types::Value>(0)
    })
    .map(|v| match v {
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Text(t) => t,
        other => format!("{other:?}"),
    })
    .unwrap_or_default()
}

pub const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Building rows for the migration 0004 tests.
pub mod chat_rows {
    use rusqlite::{Connection, params};

    pub fn session(w: &Connection, name: &str) -> i64 {
        w.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES (?1, 100, 100)",
            params![name],
        )
        .expect("session");
        w.last_insert_rowid()
    }

    pub fn persona(w: &Connection, name: &str) -> i64 {
        w.execute(
            "INSERT INTO personas(name, color) VALUES (?1, '#aabbcc')",
            params![name],
        )
        .expect("persona");
        w.last_insert_rowid()
    }

    pub fn message(
        w: &Connection,
        session: i64,
        client: Option<&str>,
        ts: i64,
    ) -> rusqlite::Result<i64> {
        w.execute(
            "INSERT INTO messages(session_id, client_id, role, body_md, ts) VALUES (?1, ?2, 'user', 'x', ?3)",
            params![session, client, ts],
        )?;
        Ok(w.last_insert_rowid())
    }

    pub fn action(w: &Connection, session: Option<i64>, persona: Option<i64>) -> i64 {
        w.execute(
            "INSERT INTO actions_log(ts, actor, session_id, persona_id, action, args_json) VALUES (1, 'agent', ?1, ?2, 'send', '{\"a\":1}')",
            params![session, persona],
        )
        .expect("log");
        w.last_insert_rowid()
    }

    pub fn n(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
    }
}

/// Versions of all code migrations in ascending order (the live `MIGRATIONS` list).
pub fn code_versions() -> Vec<i64> {
    fake_agentd::MIGRATIONS.iter().map(|m| m.version).collect()
}

/// Latest version of the code migrations.
pub fn latest_version() -> i64 {
    code_versions().last().copied().unwrap_or(0)
}

pub mod fixture;
