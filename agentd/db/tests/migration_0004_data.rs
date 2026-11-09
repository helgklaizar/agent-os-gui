//! A3-01 (DB): migration 0004 data: cascades, triggers, `CHECK` constraints, index and pagination by `id`.

mod common;

use agentd_db::user_version;
use common::migrated_to;
use rusqlite::Connection;

fn db4() -> Connection {
    let c = migrated_to(4);
    assert_eq!(user_version(&c).unwrap(), 4, "DB brought up to version 4");
    c
}

fn session(c: &Connection) -> i64 {
    c.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
        [],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn message(c: &Connection, session_id: i64, client_id: Option<&str>) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO messages(session_id, client_id, role, body_md, ts) VALUES (?1, ?2, 'user', 'x', 1)",
        rusqlite::params![session_id, client_id],
    )
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn messages_cascade_on_session_delete() {
    let c = db4();
    let (a, b) = (session(&c), session(&c));
    message(&c, a, Some("1")).unwrap();
    message(&c, a, None).unwrap();
    message(&c, b, Some("1")).unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [a])
        .unwrap();
    assert_eq!(
        count(&c, "SELECT count(*) FROM messages WHERE session_id = 1"),
        0
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM messages"),
        1,
        "others are intact"
    );
}

#[test]
fn actions_log_survives_session_delete() {
    let c = db4();
    let s = session(&c);
    c.execute(
        "INSERT INTO actions_log(ts, actor, session_id, action, args_json, undo_json) VALUES (1,'agent',?1,'shell','{\"cmd\":\"ls\"}','{\"u\":1}')",
        [s],
    )
    .unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    let (sid, args, undo): (Option<i64>, Option<String>, Option<String>) = c
        .query_row(
            "SELECT session_id, args_json, undo_json FROM actions_log",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((sid, args), (None, None));
    assert_eq!(undo.as_deref(), Some("{\"u\":1}"), "undo_json stays");
}

#[test]
fn session_delete_cleans_session_permissions() {
    let c = db4();
    let (a, b) = (session(&c), session(&c));
    for (scope, id) in [("session", a), ("session", b), ("global", 0)] {
        c.execute(
            "INSERT INTO permissions(scope, scope_id, action, mode) VALUES (?1, ?2, 'send', 'ask')",
            rusqlite::params![scope, id],
        )
        .unwrap();
    }
    c.execute("DELETE FROM sessions WHERE id = ?1", [a])
        .unwrap();
    let left: Vec<(String, i64)> = c
        .prepare("SELECT scope, scope_id FROM permissions ORDER BY scope, scope_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(left, [("global".into(), 0), ("session".into(), b)]);
}

#[test]
fn session_delete_removes_bookmarks_setting() {
    let c = db4();
    let s = session(&c);
    c.execute(
        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, '[]', 1, 1), ('browser.bookmarks.999', '[]', 1, 1)",
        [format!("browser.bookmarks.{s}")],
    )
    .unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    let keys: Vec<String> = c
        .prepare("SELECT key FROM settings")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(keys, ["browser.bookmarks.999"]);
}

#[test]
fn persona_delete_cleans_persona_permissions() {
    let c = db4();
    c.execute_batch(
        "INSERT INTO personas(id,name,color) VALUES (1,'a','#fff'),(2,'b','#fff');
         INSERT INTO permissions VALUES ('persona',1,'send','ask'),('persona',2,'send','ask');",
    )
    .unwrap();
    c.execute("DELETE FROM personas WHERE id = 1", []).unwrap();
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM permissions WHERE scope='persona' AND scope_id = 2"
        ),
        1
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM permissions WHERE scope='persona'"),
        1
    );
}

#[test]
fn finding_ref_nulled_when_action_deleted() {
    let c = db4();
    c.execute_batch(
        "INSERT INTO actions_log(id,ts,actor,action) VALUES (5,1,'agent','shell'),(6,1,'agent','shell');
         INSERT INTO observer_findings(ts,kind,severity,ref_table,ref_id,verdict) VALUES
           (1,'dangerous_cmd','high','actions_log','5','blocked'),
           (1,'dangerous_cmd','high','actions_log','6','blocked');",
    )
    .unwrap();
    c.execute("DELETE FROM actions_log WHERE id = 5", [])
        .unwrap();
    let refs: Vec<Option<String>> = c
        .prepare("SELECT ref_id FROM observer_findings ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(refs, [None, Some("6".to_string())]);
}

#[test]
fn messages_status_check() {
    let c = db4();
    let s = session(&c);
    let ins = |status: &str| {
        c.execute(
            "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES (?1,'user','x',?2,1)",
            rusqlite::params![s, status],
        )
    };
    for ok in ["streaming", "done", "stopped", "error", "interrupted"] {
        ins(ok).unwrap();
    }
    assert!(ins("pending").is_err());
    assert!(ins("").is_err());
}

#[test]
fn messages_role_check() {
    let c = db4();
    let s = session(&c);
    let ins = |role: &str| {
        c.execute(
            "INSERT INTO messages(session_id, role, body_md, ts) VALUES (?1,?2,'x',1)",
            rusqlite::params![s, role],
        )
    };
    for ok in ["user", "agent", "system", "tool"] {
        ins(ok).unwrap();
    }
    assert!(ins("admin").is_err());
}

#[test]
fn messages_unique_client_id() {
    let c = db4();
    let (a, b) = (session(&c), session(&c));
    message(&c, a, Some("x")).unwrap();
    assert!(message(&c, a, Some("x")).is_err(), "duplicate in session");
    message(&c, b, Some("x")).unwrap();
    message(&c, a, None).unwrap();
    message(&c, a, None).unwrap();
}

#[test]
fn messages_paginate_by_id_stable() {
    let c = db4();
    let s = session(&c);
    for _ in 0..3 {
        message(&c, s, None).unwrap();
    }
    let page = |before: i64, limit: i64| -> Vec<i64> {
        c.prepare(
            "SELECT id FROM messages WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
        )
        .unwrap()
        .query_map(rusqlite::params![s, before, limit], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    assert_eq!(page(i64::MAX, 2), [3, 2]);
    assert_eq!(page(2, 2), [1]);
}

#[test]
fn messages_index_used() {
    let c = db4();
    let plan: String = c
        .prepare("EXPLAIN QUERY PLAN SELECT id FROM messages WHERE session_id = 1 ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(";");
    assert!(plan.contains("idx_messages_session"), "{plan}");
}

#[test]
fn permissions_primary_key_and_mode_check() {
    let c = db4();
    let ins = |mode: &str| {
        c.execute(
            "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('global',0,'send',?1)",
            [mode],
        )
    };
    assert!(ins("maybe").is_err());
    ins("ask").unwrap();
    assert!(ins("deny").is_err(), "duplicate key");
    assert!(
        c.execute(
            "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('team',0,'send','ask')",
            []
        )
        .is_err()
    );
}
