//! `CHECK`, uniqueness, indexes and pagination of migration 0004.

mod common;

use common::chat_rows::{message, session};
use common::*;
use fake_agentd::MIGRATIONS;
use rusqlite::params;

fn fresh() -> (tempfile::TempDir, std::path::PathBuf, fake_agentd::Db) {
    let (d, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &MIGRATIONS[..3]).expect("v3"));
    let db = fake_agentd::Db::open_with(&path, &MIGRATIONS[..4]).expect("DB version 4");
    assert_eq!(
        user_version(&db.reader().expect("reader")),
        4,
        "expecting version 4"
    );
    (d, path, db)
}

fn rejected(db: &fake_agentd::Db, sql: &str) -> bool {
    db.write(|w| w.execute(sql, [])).is_err()
}

#[test]
fn messages_status_check() {
    let (_d, _p, db) = fresh();
    let s = db.write(|w| Ok(session(w, "s"))).expect("session");
    for ok in ["streaming", "done", "stopped", "error", "interrupted"] {
        let sql = format!(
            "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES ({s}, 'agent', '', '{ok}', 1)"
        );
        assert!(!rejected(&db, &sql), "status {ok} rejected");
    }
    let bad = format!(
        "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES ({s}, 'agent', '', 'weird', 1)"
    );
    assert!(
        rejected(&db, &bad),
        "a status outside the list was accepted"
    );
}

#[test]
fn messages_unique_client_id() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let a = session(w, "a");
        let b = session(w, "b");
        message(w, a, Some("c1"), 1)?;
        assert!(
            message(w, a, Some("c1"), 2).is_err(),
            "a duplicate in the same session was accepted"
        );
        message(w, b, Some("c1"), 3)?;
        message(w, a, None, 4)?;
        message(w, a, None, 5)?;
        Ok(())
    })
    .expect("scenario");
}

#[test]
fn messages_role_check() {
    let (_d, _p, db) = fresh();
    let s = db.write(|w| Ok(session(w, "s"))).expect("session");
    let bad =
        format!("INSERT INTO messages(session_id, role, body_md, ts) VALUES ({s}, 'bot', '', 1)");
    assert!(rejected(&db, &bad));
    let ok =
        format!("INSERT INTO messages(session_id, role, body_md, ts) VALUES ({s}, 'tool', '', 1)");
    assert!(!rejected(&db, &ok));
}

#[test]
fn permissions_primary_key() {
    let (_d, _p, db) = fresh();
    let sql = "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('global', 0, 'send', 'ask')";
    assert!(!rejected(&db, sql));
    assert!(rejected(&db, sql), "a duplicate primary key was accepted");
}

#[test]
fn permissions_mode_check() {
    let (_d, _p, db) = fresh();
    assert!(rejected(
        &db,
        "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('global', 0, 'send', 'maybe')"
    ));
    assert!(rejected(
        &db,
        "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('team', 0, 'send', 'ask')"
    ));
}

#[test]
fn actions_log_decision_check() {
    let (_d, _p, db) = fresh();
    assert!(rejected(
        &db,
        "INSERT INTO actions_log(ts, actor, action, decision) VALUES (1, 'agent', 'send', 'maybe')"
    ));
    for d in ["allow", "ask-approved", "ask-denied", "deny"] {
        let sql = format!(
            "INSERT INTO actions_log(ts, actor, action, decision) VALUES (1, 'agent', 'send', '{d}')"
        );
        assert!(!rejected(&db, &sql), "decision {d} rejected");
    }
}

#[test]
fn observer_verdict_check() {
    let (_d, _p, db) = fresh();
    assert!(rejected(
        &db,
        "INSERT INTO observer_findings(ts, kind, severity, verdict) VALUES (1, 'injection', 'low', 'maybe')"
    ));
    assert!(!rejected(
        &db,
        "INSERT INTO observer_findings(ts, kind, severity, verdict) VALUES (1, 'injection', 'low', 'blocked')"
    ));
}

#[test]
fn messages_index_used() {
    let (_d, _p, db) = fresh();
    let r = db.reader().expect("reader");
    let mut st = r
        .prepare("EXPLAIN QUERY PLAN SELECT * FROM messages WHERE session_id = 1 ORDER BY id DESC LIMIT 50")
        .expect("plan");
    let plan: Vec<String> = st
        .query_map([], |x| x.get::<_, String>(3))
        .expect("rows")
        .flatten()
        .collect();
    assert!(
        plan.iter().any(|l| l.contains("idx_messages_session")),
        "plan: {plan:?}"
    );
}

#[test]
fn messages_paginate_by_id_stable() {
    let (_d, _p, db) = fresh();
    let s = db.write(|w| Ok(session(w, "s"))).expect("session");
    db.write(|w| {
        for _ in 0..3 {
            message(w, s, None, 500)?;
        }
        Ok(())
    })
    .expect("messages");
    let r = db.reader().expect("reader");
    let page = |before: i64| -> Vec<i64> {
        let mut st = r
            .prepare("SELECT id FROM messages WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT 2")
            .expect("query");
        st.query_map(params![s, before], |x| x.get(0))
            .expect("rows")
            .flatten()
            .collect()
    };
    let first = page(i64::MAX);
    assert_eq!(first.len(), 2);
    let second = page(*first.last().unwrap_or(&0));
    assert_eq!(second.len(), 1);
    let mut all: Vec<i64> = first.into_iter().chain(second).collect();
    let mut sorted = all.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(all.len(), 3);
    assert_eq!(all, sorted, "descending order, without duplicates");
    all.clear();
}
