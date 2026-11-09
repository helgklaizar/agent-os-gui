//! Migration 0004 (`messages`, `permissions`, `actions_log`, `observer_findings`; db.md sections 3.2, 3.6, 3.7, 3.11).

mod common;

use common::chat_rows::n;
use common::*;
use fake_agentd::{Db, MIGRATIONS};

/// A DB at version 3, then the live `Db::open` (the "up from version 3" path).
fn upgraded_from_v3(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..3]).expect("DB version 3");
    drop(old);
    let db = Db::open_with(path, &MIGRATIONS[..4]).expect("DB version 4");
    assert_eq!(
        user_version(&db.reader().expect("reader")),
        4,
        "after opening a version 3 DB we expect version 4"
    );
    db
}

#[test]
fn migration_0004_applies_up() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v3(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2, 3, 4]);
    assert_eq!(user_version(&r), 4);
    let (name, at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 4",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "chat");
    assert!(at > 1_700_000_000, "applied_at is unix seconds UTC");
}

#[test]
fn migration_0004_idempotent() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v3(&path);
    db.write(|w| {
        common::chat_rows::session(w, "s");
        Ok(())
    })
    .expect("row");
    drop(db);
    let db = Db::open_with(&path, &MIGRATIONS[..4]).expect("DB version 4");
    let r = db.reader().expect("reader");
    assert_eq!(user_version(&r), 4);
    assert_eq!(applied_versions(&r), vec![1, 2, 3, 4]);
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 1, "data intact");
}

#[test]
fn migration_0004_schema() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v3(&path);
    let r = db.reader().expect("reader");
    for t in [
        "messages",
        "permissions",
        "actions_log",
        "observer_findings",
    ] {
        assert!(object_exists(&r, "table", t), "no table {t}");
    }
    for i in [
        "idx_messages_session",
        "idx_messages_client",
        "idx_actions_ts",
        "idx_actions_session",
        "idx_observer_ts",
    ] {
        assert!(object_exists(&r, "index", i), "no index {i}");
    }
    for t in [
        "trg_sessions_cleanup",
        "trg_personas_cleanup",
        "trg_findings_ref_actions",
    ] {
        assert!(object_exists(&r, "trigger", t), "no trigger {t}");
    }
    let cols = |t: &str| -> Vec<String> {
        let mut st = r
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{t}') ORDER BY cid"
            ))
            .expect("info");
        st.query_map([], |x| x.get(0))
            .expect("rows")
            .flatten()
            .collect()
    };
    assert_eq!(
        cols("messages"),
        [
            "id",
            "session_id",
            "client_id",
            "role",
            "author",
            "body_md",
            "status",
            "meta_json",
            "ts"
        ]
    );
    assert_eq!(cols("permissions"), ["scope", "scope_id", "action", "mode"]);
    assert_eq!(
        cols("actions_log"),
        [
            "id",
            "ts",
            "actor",
            "session_id",
            "persona_id",
            "action",
            "target",
            "args_json",
            "decision",
            "result",
            "reversible",
            "undo_json",
            "undone_at"
        ]
    );
    assert_eq!(
        cols("observer_findings"),
        [
            "id",
            "ts",
            "kind",
            "severity",
            "ref_table",
            "ref_id",
            "verdict",
            "detail"
        ]
    );
}

#[test]
fn migration_0004_foreign_keys() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v3(&path);
    db.write(|w| {
        let s = common::chat_rows::session(w, "s");
        common::chat_rows::message(w, s, Some("c1"), 1).expect("message");
        common::chat_rows::action(w, Some(s), None);
        Ok(())
    })
    .expect("fixture");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM messages"),
        1,
        "fixture inserted"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM pragma_foreign_key_check"), 0);
    let orphan = db.write(|w| common::chat_rows::message(w, 999, None, 1));
    assert!(
        orphan.is_err(),
        "a message of a nonexistent session was accepted"
    );
}

#[test]
fn migration_0004_integrity() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v3(&path);
    let r = db.reader().expect("reader");
    assert_eq!(pragma_text(&r, "integrity_check"), "ok");
}
