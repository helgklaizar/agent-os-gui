//! Migration 0002 (`accounts`, `quotas`, `quota_samples`, `indicators`; db.md sections 3.4, 3.5, 3.12): applying, idempotency, schema, keys.

mod common;
mod mig0002_common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use mig0002_common::*;

#[test]
fn migration_0002_applies_up() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2]);
    assert_eq!(user_version(&r), 2);
    let (name, at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 2",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "bar");
    assert!(at > 1_700_000_000, "applied_at is unix seconds UTC");
}

#[test]
fn migration_0002_idempotent() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| account(w, "cl1")).expect("account");
    let first: i64 = db
        .reader()
        .unwrap()
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 2",
            [],
            |x| x.get(0),
        )
        .unwrap_or(-1);
    drop(db);
    let again = Db::open_with(&path, &MIGRATIONS[..2]).expect("reopening");
    let r = again.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2]);
    assert_eq!(user_version(&r), 2);
    let second: i64 = r
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 2",
            [],
            |x| x.get(0),
        )
        .unwrap_or(-2);
    assert_eq!(
        first, second,
        "a repeated start does not rewrite the record"
    );
    assert_eq!(account_id(&r, "cl1"), 1, "data in place");
}

#[test]
fn migration_0002_schema() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    let r = db.reader().expect("reader");
    assert_eq!(
        cols(&r, "accounts"),
        vec![
            c("id", "INTEGER", false, true),
            c("slug", "TEXT", true, false),
            c("type", "TEXT", true, false),
            c("label", "TEXT", true, false),
            c("secret_ref", "TEXT", false, false),
            c("status", "TEXT", true, false),
            c("created_at", "INTEGER", true, false),
            c("deleted_at", "INTEGER", false, false),
        ]
    );
    assert_eq!(
        cols(&r, "quotas"),
        vec![
            c("account_id", "INTEGER", false, true),
            c("kind", "TEXT", true, false),
            c("window_h", "REAL", false, false),
            c("used", "REAL", false, false),
            c("rate", "REAL", false, false),
            c("load", "REAL", false, false),
            c("usage_tokens", "INTEGER", false, false),
            c("usage_cost", "REAL", false, false),
            c("limit_requests", "INTEGER", false, false),
            c("limit_tokens", "INTEGER", false, false),
            c("remaining", "REAL", false, false),
            c("reset_at", "INTEGER", false, false),
            c("updated_at", "INTEGER", true, false),
        ]
    );
    assert_eq!(
        cols(&r, "quota_samples"),
        vec![
            c("account_id", "INTEGER", true, true),
            c("ts", "INTEGER", true, true),
            c("used", "REAL", false, false),
            c("usage_tokens", "INTEGER", false, false),
            c("cost", "REAL", false, false),
        ]
    );
    assert_eq!(
        cols(&r, "indicators"),
        vec![
            c("id", "TEXT", false, true),
            c("enabled", "INTEGER", true, false),
            c("position", "INTEGER", true, false),
            c("config_json", "TEXT", true, false),
            c("last_state", "TEXT", true, false),
            c("transition_seq", "INTEGER", true, false),
            c("last_change_at", "INTEGER", false, false),
        ]
    );
    // db.md sets no indexes beyond primary keys: only SQLite auto-indexes are allowed
    let mut stmt = r
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name IN ('accounts','quotas','quota_samples','indicators')",
        )
        .expect("index query");
    let names: Vec<String> = stmt
        .query_map([], |x| x.get(0))
        .expect("indexes")
        .flatten()
        .collect();
    assert!(
        names.iter().all(|n| n.starts_with("sqlite_autoindex_")),
        "extra indexes: {names:?}"
    );
    // DEFAULT and CHECK from the DDL
    let sql: String = r
        .query_row(
            "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE name IN ('accounts','quotas','indicators')",
            [],
            |x| x.get(0),
        )
        .unwrap_or_default();
    for needle in [
        "DEFAULT 'disconnected'",
        "CHECK (status IN ('connected','disconnected','qr','error'))",
        "CHECK (kind IN ('api_key','local'))",
        "ON DELETE CASCADE",
        "CHECK (last_state IN ('ok','warn','bad','nodata','unknown'))",
    ] {
        assert!(sql.contains(needle), "the DDL lacks `{needle}`");
    }
}

#[test]
fn migration_0002_foreign_keys() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    db.write(|w| {
        account(w, "cl1")?;
        w.execute(
            "INSERT INTO quotas(account_id, kind, window_h, used, rate, updated_at) VALUES (1, 'api_key', 5, 0.62, 1.4, 100)",
            [],
        )?;
        w.execute(
            "INSERT INTO quota_samples(account_id, ts, used) VALUES (1, 100, 0.5)",
            [],
        )
    })
    .expect("fixture");
    let r = db.reader().expect("reader");
    let violations: i64 = r
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |x| {
            x.get(0)
        })
        .unwrap_or(-1);
    assert_eq!(violations, 0, "foreign_key_check must be empty");
    let rows: i64 = r
        .query_row("SELECT count(*) FROM quotas", [], |x| x.get(0))
        .unwrap_or(-1);
    assert_eq!(rows, 1, "fixture inserted");
    // an orphaned row is rejected by the foreign key
    let orphan = db.write(|w| {
        w.execute(
            "INSERT INTO quota_samples(account_id, ts, used) VALUES (999, 1, 0.1)",
            [],
        )
    });
    assert!(orphan.is_err(), "a sample without an account was accepted");
}

#[test]
fn migration_0002_integrity() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v1(&path);
    let r = db.reader().expect("reader");
    assert_eq!(pragma_text(&r, "integrity_check"), "ok");
    assert!(
        object_exists(&r, "table", "quota_samples"),
        "0002 tables created"
    );
}
