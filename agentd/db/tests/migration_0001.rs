//! A1-02: migration 0001 (db.md sections 3.12, 3.13) through the real runner.

mod common;

use agentd_db::{MigrateOptions, migrate, user_version};
use common::mem;
use rusqlite::Connection;

fn fresh() -> Connection {
    let mut c = mem();
    migrate(&mut c, common::first(), &MigrateOptions::default()).expect("migrate");
    assert_eq!(
        user_version(&c).unwrap(),
        1,
        "after the migration the DB is version 1"
    );
    c
}

fn insert_token(
    c: &Connection,
    hash: &str,
    scope: &str,
    kind: &str,
    allow: Option<&str>,
) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO api_token(token_hash, label, scope, kind, allowlist_json, created_at) VALUES (?1, 'x', ?2, ?3, ?4, 1000)",
        rusqlite::params![hash, scope, kind, allow],
    )
}

const H1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const H2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn migration_0001_applies_up() {
    let mut c = mem();
    assert_eq!(user_version(&c).unwrap(), 0);
    let out = migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    assert_eq!(out.applied, vec![1]);
    assert_eq!(user_version(&c).unwrap(), 1);
    let versions: Vec<i64> = c
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(versions, vec![1]);
}

#[test]
fn migration_0001_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let backups = dir.path().join("backups");
    let opts = MigrateOptions {
        backup_dir: Some(backups.clone()),
        ..MigrateOptions::default()
    };
    let mut c = common::file_conn(dir.path());
    migrate(&mut c, common::first(), &opts).unwrap();
    assert_eq!(
        user_version(&c).unwrap(),
        1,
        "after the first migration the DB is version 1"
    );
    let before: i64 = c
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let out = migrate(&mut c, common::first(), &opts).unwrap();
    assert!(out.applied.is_empty(), "a repeated start applies nothing");
    assert!(out.snapshots.is_empty());
    assert!(!backups.exists(), "no snapshot is created");
    assert_eq!(user_version(&c).unwrap(), 1);
    let after: i64 = c
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn migration_0001_schema() {
    let c = fresh();
    let names: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(names, vec!["api_token", "schema_migrations", "settings"]);
    let idx: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_api_token_active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(idx.contains("WHERE revoked_at IS NULL"), "{idx}");
    let triggers: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(triggers, 2);
}

#[test]
fn migration_0001_integrity() {
    let c = fresh();
    let fk: i64 = c
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(fk, 0);
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
}

#[test]
fn api_token_check_constraints() {
    let c = fresh();
    assert!(
        insert_token(&c, H1, "admin", "external", None).is_err(),
        "external cannot be admin"
    );
    assert!(
        insert_token(&c, H1, "chat", "agent", None).is_err(),
        "agent without allowlist_json"
    );
    assert!(
        insert_token(&c, H1, "chat", "module", Some("not json")).is_err(),
        "allowlist_json is not JSON"
    );
    assert!(
        insert_token(&c, H1, "admin", "module", Some("[]")).is_err(),
        "module cannot be admin"
    );
    assert!(
        insert_token(&c, "abc", "chat", "external", None).is_err(),
        "hash is not 64 characters"
    );
    let upper = "A".repeat(64);
    assert!(
        insert_token(&c, &upper, "chat", "external", None).is_err(),
        "hash is not lowercase hex"
    );
    assert!(insert_token(&c, H1, "chat", "external", None).is_ok());
    assert!(
        insert_token(&c, H1, "chat", "external", None).is_err(),
        "hash is unique"
    );
    assert!(insert_token(&c, H2, "chat", "agent", Some("[\"GET /v1/health\"]")).is_ok());
}

#[test]
fn api_token_revoke_final_and_hash_immutable() {
    let c = fresh();
    insert_token(&c, H1, "chat", "external", None).unwrap();
    c.execute(
        "UPDATE api_token SET revoked_at = 2000 WHERE token_hash = ?1",
        [H1],
    )
    .unwrap();
    assert!(
        c.execute(
            "UPDATE api_token SET revoked_at = 3000 WHERE token_hash = ?1",
            [H1]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "UPDATE api_token SET revoked_at = NULL WHERE token_hash = ?1",
            [H1]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "UPDATE api_token SET token_hash = ?1 WHERE token_hash = ?2",
            [H2, H1]
        )
        .is_err()
    );
}
