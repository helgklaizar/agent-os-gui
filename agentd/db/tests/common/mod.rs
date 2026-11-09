//! Shared helpers for DB tests.
#![allow(dead_code)]

use agentd_db::{MIGRATIONS, Migration};
use rusqlite::Connection;

pub fn mem() -> Connection {
    let c = Connection::open_in_memory().expect("open");
    c.pragma_update(None, "foreign_keys", true).expect("fk");
    c
}

pub fn file_conn(dir: &std::path::Path) -> Connection {
    let c = Connection::open(dir.join("sessions.db")).expect("open");
    c.pragma_update(None, "foreign_keys", true).expect("fk");
    c
}

/// Only product migration 0001: the runner and 0001 tests do not depend on later migrations.
pub fn first() -> &'static [Migration] {
    &MIGRATIONS[..1]
}

/// Product migration 0001 plus synthetic ones for the runner tests.
pub fn with_extra(extra: &[Migration]) -> Vec<Migration> {
    let mut v = first().to_vec();
    v.extend_from_slice(extra);
    v
}

pub const fn data_migration(version: i64, name: &'static str, sql: &'static str) -> Migration {
    Migration {
        version,
        name,
        sql,
        mutates_data: true,
        recreates_tables: false,
    }
}

/// A DB with the first `n` product migrations (the real runner).
pub fn migrated_to(n: usize) -> Connection {
    let mut c = mem();
    let set: Vec<Migration> = MIGRATIONS.iter().take(n).copied().collect();
    agentd_db::migrate(&mut c, &set, &agentd_db::MigrateOptions::default()).expect("migrate");
    c
}

/// Normalized schema: `sql` from `sqlite_master`, without comments and extra whitespace.
pub fn normalized_schema(sql: &str) -> Vec<String> {
    let c = mem();
    c.execute_batch(sql).unwrap();
    let mut stmt = c
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.into_iter()
        .map(|s| {
            s.lines()
                .map(|l| l.split("--").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn names(c: &Connection, kind: &str) -> Vec<String> {
    c.prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name")
        .unwrap()
        .query_map([kind], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
