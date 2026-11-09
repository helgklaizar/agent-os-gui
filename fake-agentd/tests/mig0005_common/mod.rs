//! Shared by the migration 0005 tests: a DB at version 4 and inserts of `events`, `accounts`, `gw_messages` rows.
#![allow(dead_code)]

use fake_agentd::{Db, MIGRATIONS};
use rusqlite::{Connection, params};

use crate::common::*;

/// A DB at version 4, then the live `Db::open` (the "up from version 4" path).
pub fn upgraded_from_v4(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..4]).expect("DB version 4");
    drop(old);
    let db = Db::open_with(path, &MIGRATIONS[..5]).expect("DB version 5");
    assert_eq!(
        user_version(&db.reader().expect("reader")),
        5,
        "after opening a version 4 DB we expect version 5"
    );
    db
}

pub fn fresh() -> (tempfile::TempDir, Db) {
    let (d, path) = temp_db_path();
    let db = upgraded_from_v4(&path);
    (d, db)
}

pub fn n(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

pub fn session(w: &Connection, name: &str) -> i64 {
    w.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES (?1, 100, 100)",
        params![name],
    )
    .expect("session");
    w.last_insert_rowid()
}

pub fn event(
    w: &Connection,
    kind: &str,
    sub: &str,
    session: Option<i64>,
    dedup: Option<&str>,
) -> rusqlite::Result<usize> {
    w.execute(
        "INSERT OR IGNORE INTO events(kind, sub, text, session_id, dedup_key, ts) VALUES (?1, ?2, 'x', ?3, ?4, 100)",
        params![kind, sub, session, dedup],
    )
}

pub fn account(w: &Connection, slug: &str) -> i64 {
    w.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'wa', 'x', 1)",
        params![slug],
    )
    .expect("account");
    w.last_insert_rowid()
}

pub fn gw_message(w: &Connection, account: i64, ext: &str) -> rusqlite::Result<i64> {
    w.execute(
        "INSERT INTO gw_messages(account_id, ext_id, direction, body, ts) VALUES (?1, ?2, 'in', 'hi', 100)",
        params![account, ext],
    )?;
    Ok(w.last_insert_rowid())
}

pub fn indicator(w: &Connection, id: &str, state: &str) {
    w.execute(
        "INSERT INTO indicators(id, last_state) VALUES (?1, ?2)",
        params![id, state],
    )
    .expect("indicator");
}
