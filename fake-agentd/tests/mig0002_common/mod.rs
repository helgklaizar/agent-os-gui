//! Shared by the migration 0002 tests: a DB at version 2, column description, account insert and lookup.
#![allow(dead_code)]

use fake_agentd::{Db, MIGRATIONS};
use rusqlite::{Connection, params};

use crate::common::*;

pub type Col = (String, String, bool, bool);

pub fn cols(conn: &Connection, table: &str) -> Vec<Col> {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return Vec::new();
    };
    stmt.query_map([], |r| {
        Ok((
            r.get(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)? == 1,
            r.get::<_, i64>(5)? > 0,
        ))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

pub fn c(name: &str, ty: &str, notnull: bool, pk: bool) -> Col {
    (name.to_string(), ty.to_string(), notnull, pk)
}

/// A DB at version 1, then opened by the live `Db::open` (the "up from version 1" path).
pub fn upgraded_from_v1(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..1]).expect("DB version 1");
    drop(old);
    let db = Db::open_with(path, &MIGRATIONS[..2]).expect("DB version 2");
    let version = user_version(&db.reader().expect("reader"));
    assert_eq!(
        version, 2,
        "after opening a version 1 DB we expect version 2"
    );
    db
}

pub fn account(w: &Connection, slug: &str) -> rusqlite::Result<usize> {
    w.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'anthropic', 'Claude · key-1', 100)",
        params![slug],
    )
}

pub fn account_id(conn: &Connection, slug: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM accounts WHERE slug = ?1",
        params![slug],
        |r| r.get(0),
    )
    .unwrap_or(-1)
}
