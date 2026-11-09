//! Migration 0001 (`schema_migrations`, `settings`, `api_token`) and its DDL (db.md sections 3.12, 3.13).

mod common;

use common::*;
use rusqlite::Connection;

/// Migration 0001 is checked on a version 1 DB: the 0002 set is tested separately in `migration_0002.rs`.
fn open_v1(path: &std::path::Path) -> fake_agentd::Db {
    fake_agentd::Db::open_with(path, &fake_agentd::MIGRATIONS[..1]).expect("open a version 1 DB")
}

fn cols(conn: &Connection, table: &str) -> Vec<(String, String, bool, bool)> {
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

fn c(name: &str, ty: &str, notnull: bool, pk: bool) -> (String, String, bool, bool) {
    (name.to_string(), ty.to_string(), notnull, pk)
}

#[test]
fn migration_0001_applies_up_from_empty() {
    let (_d, path) = temp_db_path();
    let db = open_v1(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1]);
    assert_eq!(user_version(&r), 1);
    let (name, applied_at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 1",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "init");
    assert!(applied_at > 1_700_000_000, "applied_at is unix seconds UTC");
}

#[test]
fn migration_0001_idempotent() {
    let (_d, path) = temp_db_path();
    let db = open_v1(&path);
    db.write(|w| {
        w.execute("INSERT INTO settings(key, value_json, updated_at) VALUES ('theme', '\"dark\"', 5)", [])?;
        w.execute("INSERT INTO api_token(token_hash, label, scope, created_at) VALUES (?1, 'GUI', 'chat', 10)", [HASH_A])
    })
    .expect("data");
    let before: i64 = db
        .reader()
        .unwrap()
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    drop(db);

    let db = open_v1(&path);
    let r = db.reader().unwrap();
    assert_eq!(applied_versions(&r), vec![1]);
    assert_eq!(user_version(&r), 1);
    let after: i64 = r
        .query_row(
            "SELECT applied_at FROM schema_migrations WHERE version = 1",
            [],
            |x| x.get(0),
        )
        .unwrap_or(-2);
    assert_eq!(
        before, after,
        "a repeated start does not rewrite the version row"
    );
    assert_ne!(before, -1);
    let settings: i64 = r
        .query_row("SELECT count(*) FROM settings", [], |x| x.get(0))
        .unwrap_or(-1);
    let tokens: i64 = r
        .query_row("SELECT count(*) FROM api_token", [], |x| x.get(0))
        .unwrap_or(-1);
    assert_eq!((settings, tokens), (1, 1));
}

#[test]
fn migration_0001_schema() {
    let (_d, path) = temp_db_path();
    let db = open_v1(&path);
    let r = db.reader().unwrap();

    assert_eq!(
        cols(&r, "schema_migrations"),
        vec![
            c("version", "INTEGER", false, true),
            c("name", "TEXT", true, false),
            c("applied_at", "INTEGER", true, false)
        ]
    );
    assert_eq!(
        cols(&r, "settings"),
        vec![
            c("key", "TEXT", true, true),
            c("value_json", "TEXT", true, false),
            c("rev", "INTEGER", true, false),
            c("updated_at", "INTEGER", true, false)
        ]
    );
    assert_eq!(
        cols(&r, "api_token"),
        vec![
            c("id", "INTEGER", false, true),
            c("token_hash", "TEXT", true, false),
            c("label", "TEXT", true, false),
            c("scope", "TEXT", true, false),
            c("kind", "TEXT", true, false),
            c("allowlist_json", "TEXT", false, false),
            c("created_by", "INTEGER", false, false),
            c("created_at", "INTEGER", true, false),
            c("expires_at", "INTEGER", false, false),
            c("revoked_at", "INTEGER", false, false),
            c("last_used_at", "INTEGER", false, false),
        ]
    );
    let sql = |name: &str| -> String {
        r.query_row(
            "SELECT sql FROM sqlite_master WHERE name = ?1",
            [name],
            |x| x.get(0),
        )
        .unwrap_or_default()
    };
    assert!(
        sql("settings").contains("WITHOUT ROWID"),
        "settings — WITHOUT ROWID"
    );
    assert!(
        sql("api_token").contains("AUTOINCREMENT"),
        "ids are not reused"
    );
    assert!(object_exists(&r, "index", "idx_api_token_active"));
    assert!(object_exists(&r, "trigger", "trg_api_token_no_unrevoke"));
    assert!(object_exists(&r, "trigger", "trg_api_token_hash_immutable"));
    assert!(
        !object_exists(&r, "table", "indicators"),
        "indicators is created by a later migration"
    );

    db.write(|w| {
        w.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('k', '1', 1)",
            [],
        )
    })
    .expect("insert without rev");
    let rev: i64 = db
        .reader()
        .unwrap()
        .query_row("SELECT rev FROM settings WHERE key = 'k'", [], |x| x.get(0))
        .unwrap_or(-1);
    assert_eq!(rev, 1, "rev defaults to 1");
}

#[test]
fn migration_0001_foreign_key_and_integrity() {
    let (_d, path) = temp_db_path();
    let db = open_v1(&path);
    db.write(|w| {
        w.execute("INSERT INTO api_token(token_hash, label, scope, kind, created_at) VALUES (?1, 'admin', 'admin', 'gui', 1)", [HASH_A])?;
        w.execute("INSERT INTO api_token(token_hash, label, scope, created_by, created_at) VALUES (?1, 'phone', 'read', 1, 2)", [HASH_B])
    })
    .expect("tokens with created_by");
    let r = db.reader().unwrap();
    assert!(object_exists(&r, "table", "api_token"));
    let violations: i64 = r
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |x| {
            x.get(0)
        })
        .unwrap_or(-1);
    assert_eq!(violations, 0);
    let ic: Vec<String> = r
        .prepare("PRAGMA integrity_check")
        .and_then(|mut s| {
            s.query_map([], |x| x.get(0))
                .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();
    assert_eq!(ic, vec!["ok".to_string()]);
}

#[test]
fn migration_0001_created_by_set_null_on_delete() {
    let (_d, path) = temp_db_path();
    let db = open_v1(&path);
    db.write(|w| {
        w.execute("INSERT INTO api_token(token_hash, label, scope, kind, created_at) VALUES (?1, 'admin', 'admin', 'gui', 1)", [HASH_A])?;
        w.execute("INSERT INTO api_token(token_hash, label, scope, created_by, created_at) VALUES (?1, 'phone', 'read', 1, 2)", [HASH_B])?;
        w.execute("DELETE FROM api_token WHERE id = 1", [])
    })
    .expect("deleting the issuing token");
    let by: Option<i64> = db
        .reader()
        .unwrap()
        .query_row("SELECT created_by FROM api_token WHERE id = 2", [], |x| {
            x.get(0)
        })
        .unwrap_or(Some(-1));
    assert_eq!(
        by, None,
        "ON DELETE SET NULL works only with foreign_keys = ON"
    );
}

/// The DDL snapshot in the migration matches `db.md` (no schema copies: the document is the source of truth).
#[test]
fn migration_0001_matches_db_md() {
    fn norm(s: &str) -> String {
        let no_comments: Vec<&str> = s
            .lines()
            .map(|l| l.split("--").next().unwrap_or(""))
            .collect();
        no_comments
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sql = std::fs::read_to_string(root.join("../agentd/migrations/0001_init.sql"))
        .unwrap_or_default();
    let doc = norm(&std::fs::read_to_string(root.join("../docs/reference/db.md")).expect("db.md"));
    let mut chunks: Vec<String> = Vec::new();
    for line in sql.lines() {
        if line.starts_with("CREATE ") {
            chunks.push(String::new());
        }
        if let Some(last) = chunks.last_mut() {
            last.push_str(line);
            last.push('\n');
        }
    }
    assert_eq!(chunks.len(), 6, "3 tables, 1 index, 2 triggers");
    for chunk in chunks {
        let n = norm(&chunk);
        assert!(
            doc.contains(&n),
            "db.md has no byte-for-byte (up to whitespace and comments) DDL: {n}"
        );
    }
}
