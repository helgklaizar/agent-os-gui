//! Snapshot before a data-changing migration (db.md section 6), recreating tables with `foreign_keys=OFF`
//! and rollback on orphans, incompatible DB version. The schema against db.md is held by `schema_s9.rs`.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::*;
use fake_agentd::{Config, DbError, FakeAgentd, MIGRATIONS, Migration, migrate};
use rusqlite::Connection;

const M1: Migration = Migration {
    version: 1,
    mutates_data: false,
    name: "init",
    sql: "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);\n\
          CREATE TABLE parent (id INTEGER PRIMARY KEY, v TEXT NOT NULL);\n\
          CREATE TABLE child (id INTEGER PRIMARY KEY, p INTEGER NOT NULL REFERENCES parent(id));\n\
          INSERT INTO parent(id, v) VALUES (1, 'a');\n\
          INSERT INTO child(id, p) VALUES (1, 1);",
};

fn snapshots(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir.join("migration-backups"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn migration_changing_data_makes_snapshot() {
    let (dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &[M1]).expect("v1"));
    let m2 = Migration {
        version: 2,
        mutates_data: true,
        name: "data",
        sql: "UPDATE parent SET v = 'b';",
    };
    drop(fake_agentd::Db::open_with(&path, &[M1, m2]).expect("v2"));
    let root = path.parent().expect("dir");
    assert_eq!(snapshots(root), vec!["pre-migration-2.db".to_string()]);
    let snap = root.join("migration-backups/pre-migration-2.db");
    let mode = fs::metadata(&snap).expect("snap").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "snapshot file 0600");
    let dmode = fs::metadata(snap.parent().expect("d"))
        .expect("d")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dmode, 0o700, "snapshot directory 0700");
    let old = Connection::open(&snap).expect("open snap");
    let v: String = old
        .query_row("SELECT v FROM parent", [], |r| r.get(0))
        .expect("v");
    assert_eq!(v, "a", "snapshot taken before the migration");
    assert_eq!(user_version(&old), 1);
    drop(dir);
}

#[test]
fn migration_without_data_change_makes_no_snapshot() {
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &[M1]).expect("v1"));
    let m2 = Migration {
        version: 2,
        mutates_data: false,
        name: "ddl",
        sql: "CREATE TABLE extra (id INTEGER PRIMARY KEY);",
    };
    drop(fake_agentd::Db::open_with(&path, &[M1, m2]).expect("v2"));
    assert!(snapshots(path.parent().expect("dir")).is_empty());
}

#[test]
fn migration_rebuilding_table_runs_with_fk_off_and_checks() {
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &[M1]).expect("v1"));
    // recreating the parent: with foreign_keys=ON DROP TABLE parent would fail on child rows
    let rebuild = Migration {
        version: 2,
        mutates_data: true,
        name: "rebuild",
        sql: "\
              CREATE TABLE parent_new (id INTEGER PRIMARY KEY, v TEXT NOT NULL CHECK (length(v) > 0));\n\
              INSERT INTO parent_new SELECT id, v FROM parent;\n\
              DROP TABLE parent;\n\
              ALTER TABLE parent_new RENAME TO parent;",
    };
    drop(fake_agentd::Db::open_with(&path, &[M1, rebuild]).expect("recreation"));
    let conn = Connection::open(&path).expect("open");
    let fk: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .expect("fk");
    assert_eq!(fk, 0);
    assert_eq!(user_version(&conn), 2);
}

#[test]
fn migration_leaving_orphans_is_rolled_back() {
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &[M1]).expect("v1"));
    let bad = Migration {
        version: 2,
        mutates_data: true,
        name: "orphan",
        sql: "PRAGMA defer_foreign_keys = 0;\nDELETE FROM parent;",
    };
    let err = fake_agentd::Db::open_with(&path, &[M1, bad]).err();
    assert!(
        matches!(err, Some(DbError::ForeignKeyViolation { version: 2 })),
        "orphaned rows roll the migration back: {err:?}"
    );
    let conn = Connection::open(&path).expect("open");
    assert_eq!(user_version(&conn), 1);
    assert_eq!(applied_versions(&conn), vec![1]);
    let n: i64 = conn
        .query_row("SELECT count(*) FROM parent", [], |r| r.get(0))
        .expect("n");
    assert_eq!(n, 1, "data untouched");
}

#[test]
fn migrate_in_memory_database_makes_no_snapshot_dir() {
    let mut mem = Connection::open_in_memory().expect("memory");
    let m2 = Migration {
        version: 2,
        mutates_data: true,
        name: "data",
        sql: "UPDATE parent SET v = 'b';",
    };
    migrate(&mut mem, &[M1, m2]).expect("in memory without a snapshot");
    assert_eq!(user_version(&mem), 2);
}

#[tokio::test]
async fn incompatible_version_refuses_start_and_reports_error_event() {
    let (_d, path) = temp_db_path();
    drop(open(&path));
    let known = MIGRATIONS.last().map_or(0, |m| m.version);
    let raw = Connection::open(&path).expect("open");
    raw.pragma_update(None, "user_version", known + 1)
        .expect("uv");
    drop(raw);

    let started = FakeAgentd::start(Config::temp().db(path.clone())).await;
    assert!(
        started.is_err(),
        "DB newer than the code: fake-agentd does not start"
    );

    let events = path.with_extension("db.events.jsonl");
    let text = fs::read_to_string(&events).unwrap_or_default();
    let line = text.lines().last().unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
    assert_eq!(v["type"], "error", "error event: {text:?}");
    assert_eq!(v["code"], "db_version_unsupported");
    let raw = Connection::open(&path).expect("open");
    assert_eq!(user_version(&raw), known + 1, "DB untouched");
}

/// Snapshot directory after upgrading a DB from version `from` to the latest (the real mock migration chain).
fn snapshots_after_upgrade_from(from: usize) -> Vec<String> {
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &MIGRATIONS[..from]).expect("old version"));
    drop(fake_agentd::Db::open(&path).expect("upgrade"));
    snapshots(path.parent().expect("dir"))
}

#[test]
fn real_migration_0009_recreating_api_token_makes_snapshot() {
    // db.md section 6 and the core catalog: 0009 recreates `api_token` moving data - a snapshot before it
    let m9 = MIGRATIONS
        .iter()
        .find(|m| m.version == 9)
        .expect("0009 exists");
    assert!(m9.mutates_data, "0009 is marked as data-changing");
    assert_eq!(
        snapshots_after_upgrade_from(8),
        vec!["pre-migration-9.db".to_string()]
    );
    // from version 1 there is also one snapshot: the other migrations do not change data
    assert_eq!(
        snapshots_after_upgrade_from(1),
        vec!["pre-migration-9.db".to_string()]
    );
}

#[test]
fn real_migrations_other_than_0009_make_no_snapshot() {
    for m in MIGRATIONS.iter().filter(|m| m.version != 9) {
        assert!(
            !m.mutates_data,
            "migration {} does not change data",
            m.version
        );
    }
    // an empty DB needs no snapshot: there is nothing to save
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open(&path).expect("from scratch"));
    assert!(snapshots(path.parent().expect("dir")).is_empty());
}

#[test]
fn snapshot_of_0009_holds_the_tokens_as_they_were() {
    let (_dir, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &MIGRATIONS[..8]).expect("v8"));
    let raw = Connection::open(&path).expect("open");
    raw.execute(
        "INSERT INTO api_token(token_hash, label, scope, kind, created_at) VALUES (?1, 't', 'read', 'external', 1)",
        ["a".repeat(64)],
    )
    .expect("token");
    drop(raw);
    drop(fake_agentd::Db::open(&path).expect("upgrade"));
    let snap = Connection::open(
        path.parent()
            .expect("dir")
            .join("migration-backups/pre-migration-9.db"),
    )
    .expect("snapshot");
    let n: i64 = snap
        .query_row("SELECT count(*) FROM api_token", [], |r| r.get(0))
        .expect("n");
    assert_eq!(n, 1, "the token is in the snapshot before the migration");
    assert_eq!(
        user_version(&snap),
        8,
        "the snapshot was taken at the version before the migration"
    );
    let now = Connection::open(&path).expect("open");
    let n: i64 = now
        .query_row("SELECT count(*) FROM api_token", [], |r| r.get(0))
        .expect("n");
    assert_eq!(n, 1, "after the migration the token is in place");
}
