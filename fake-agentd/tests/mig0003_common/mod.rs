//! Shared by the migration 0003 tests: a DB at version 3 and inserts of `folders`, `sessions`, `personas`, `accounts` rows.
#![allow(dead_code)]

use fake_agentd::{Db, MIGRATIONS};
use rusqlite::{Connection, params};

use crate::common::*;

/// A DB at version 2, then the live `Db::open` (the "up from version 2" path).
pub fn upgraded_from_v2(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..2]).expect("DB version 2");
    drop(old);
    let db = Db::open_with(path, &MIGRATIONS[..3]).expect("DB version 3");
    assert_eq!(
        user_version(&db.reader().expect("reader")),
        3,
        "after opening a version 2 DB we expect version 3"
    );
    db
}

pub fn fresh() -> (tempfile::TempDir, Db) {
    let (d, path) = temp_db_path();
    let db = upgraded_from_v2(&path);
    (d, db)
}

pub fn n(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

pub fn folder(w: &Connection, parent: Option<i64>, name: &str, persona: Option<i64>) -> i64 {
    w.execute(
        "INSERT INTO folders(parent_id, name, persona_id, created_at) VALUES (?1, ?2, ?3, 100)",
        params![parent, name, persona],
    )
    .expect("folder");
    w.last_insert_rowid()
}

pub fn session(w: &Connection, folder: Option<i64>, parent: Option<i64>, name: &str) -> i64 {
    w.execute(
        "INSERT INTO sessions(folder_id, parent_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, 100, 100)",
        params![folder, parent, name],
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

pub fn account(w: &Connection, slug: &str) -> i64 {
    w.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'anthropic', 'x', 1)",
        params![slug],
    )
    .expect("account");
    w.last_insert_rowid()
}
