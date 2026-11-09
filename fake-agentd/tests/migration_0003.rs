//! Migration 0003 (`personas`, `persona_accounts`, `folders`, `sessions`; db.md sections 3.1, 3.4, 5): applying, idempotency, schema, foreign keys.

mod common;
mod mig0003_common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use mig0003_common::*;
use rusqlite::Connection;

#[test]
fn migration_0003_applies_up() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v2(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2, 3]);
    assert_eq!(user_version(&r), 3);
    let (name, at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 3",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "sessions");
    assert!(at > 1_700_000_000, "applied_at is unix seconds UTC");
}

#[test]
fn migration_0003_idempotent() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v2(&path);
    db.write(|w| {
        let f = folder(w, None, "Clients", None);
        session(w, Some(f), None, "Release");
        Ok(())
    })
    .expect("data");
    let first = n(
        &db.reader().unwrap(),
        "SELECT applied_at FROM schema_migrations WHERE version = 3",
    );
    drop(db);
    let again = Db::open_with(&path, &MIGRATIONS[..3]).expect("DB version 3");
    let r = again.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2, 3]);
    assert_eq!(user_version(&r), 3);
    assert_eq!(
        first,
        n(
            &r,
            "SELECT applied_at FROM schema_migrations WHERE version = 3"
        ),
        "a repeated start does not rewrite the record"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 1, "data in place");
}

type Col = (String, String, bool, bool);

fn cols(conn: &Connection, table: &str) -> Vec<Col> {
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

fn c(name: &str, ty: &str, notnull: bool, pk: bool) -> Col {
    (name.to_string(), ty.to_string(), notnull, pk)
}

#[test]
fn migration_0003_schema() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert_eq!(
        cols(&r, "personas"),
        vec![
            c("id", "INTEGER", false, true),
            c("name", "TEXT", true, false),
            c("color", "TEXT", true, false),
            c("model_acc", "INTEGER", false, false),
            c("style", "TEXT", true, false),
            c("lang", "TEXT", true, false),
            c("signature", "TEXT", true, false),
            c("mem_mode", "TEXT", true, false),
            c("notif_mode", "TEXT", true, false),
            c("work_hours", "TEXT", true, false),
            c("quiet_hours", "TEXT", true, false),
            c("deleted_at", "INTEGER", false, false),
        ]
    );
    assert_eq!(
        cols(&r, "persona_accounts"),
        vec![
            c("persona_id", "INTEGER", true, true),
            c("account_id", "INTEGER", true, true),
        ]
    );
    assert_eq!(
        cols(&r, "folders"),
        vec![
            c("id", "INTEGER", false, true),
            c("parent_id", "INTEGER", false, false),
            c("name", "TEXT", true, false),
            c("persona_id", "INTEGER", false, false),
            c("open", "INTEGER", true, false),
            c("created_at", "INTEGER", true, false),
        ]
    );
    assert_eq!(
        cols(&r, "sessions"),
        vec![
            c("id", "INTEGER", false, true),
            c("folder_id", "INTEGER", false, false),
            c("parent_id", "INTEGER", false, false),
            c("name", "TEXT", true, false),
            c("persona_id", "INTEGER", false, false),
            c("path", "TEXT", false, false),
            c("cfg_json", "TEXT", true, false),
            c("unread", "INTEGER", true, false),
            c("archived", "INTEGER", true, false),
            c("created_at", "INTEGER", true, false),
            c("updated_at", "INTEGER", true, false),
        ]
    );
    for idx in [
        "idx_sessions_folder",
        "idx_sessions_parent",
        "idx_folders_parent",
    ] {
        assert!(object_exists(&r, "index", idx), "no index {idx}");
    }
    for trg in [
        "trg_folders_no_cycle",
        "trg_folders_depth_ins",
        "trg_folders_depth_upd",
        "trg_sessions_thread_ins",
        "trg_sessions_thread_upd",
    ] {
        assert!(object_exists(&r, "trigger", trg), "no trigger {trg}");
    }
    let sql: String = r
        .query_row(
            "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE name IN ('personas','folders','sessions')",
            [],
            |x| x.get(0),
        )
        .unwrap_or_default();
    for needle in [
        "CHECK (mem_mode IN ('own','shared','none'))",
        "CHECK (notif_mode IN ('all','important','digest','off'))",
        "CHECK (parent_id IS NULL OR parent_id <> id)",
        "ON DELETE CASCADE",
        "ON DELETE SET NULL",
        "AUTOINCREMENT",
    ] {
        assert!(sql.contains(needle), "the DDL lacks `{needle}`");
    }
}

#[test]
fn migration_0003_foreign_keys() {
    let (_d, db) = fresh();
    db.write(|w| {
        let p = persona(w, "Work");
        let f = folder(w, None, "Clients", Some(p));
        let s = session(w, Some(f), None, "Release");
        session(w, Some(f), Some(s), "Review");
        Ok(())
    })
    .expect("fixture");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM pragma_foreign_key_check"), 0);
    assert_eq!(
        n(&r, "SELECT count(*) FROM sessions"),
        2,
        "fixture inserted"
    );
    let orphan = db.write(|w| {
        w.execute(
            "INSERT INTO sessions(folder_id, name, created_at, updated_at) VALUES (999, 'x', 1, 1)",
            [],
        )
    });
    assert!(
        orphan.is_err(),
        "a session in a nonexistent folder was accepted"
    );
}

#[test]
fn migration_0003_integrity() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert_eq!(pragma_text(&r, "integrity_check"), "ok");
    assert!(object_exists(&r, "table", "persona_accounts"));
}
