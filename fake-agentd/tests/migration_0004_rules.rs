//! Cascades, triggers, `CHECK`, indexes of migration 0004.

mod common;

use common::chat_rows::{action, message, n, persona, session};
use common::*;
use fake_agentd::{Config, FakeAgentd, MIGRATIONS};
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

#[test]
fn messages_cascade_on_session_delete() {
    let (_d, _p, db) = fresh();
    let (a, b) = db
        .write(|w| {
            let a = session(w, "a");
            let b = session(w, "b");
            message(w, a, None, 1)?;
            message(w, a, None, 2)?;
            message(w, b, None, 3)?;
            w.execute("DELETE FROM sessions WHERE id = ?1", params![a])?;
            Ok((a, b))
        })
        .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            &format!("SELECT count(*) FROM messages WHERE session_id = {a}")
        ),
        0
    );
    assert_eq!(
        n(
            &r,
            &format!("SELECT count(*) FROM messages WHERE session_id = {b}")
        ),
        1
    );
}

#[test]
fn actions_log_survives_session_delete() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let s = session(w, "s");
        action(w, Some(s), None);
        w.execute("DELETE FROM sessions WHERE id = ?1", params![s])?;
        Ok(())
    })
    .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM actions_log"),
        1,
        "the record stayed"
    );
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM actions_log WHERE session_id IS NULL"
        ),
        1
    );
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM actions_log WHERE args_json IS NULL"
        ),
        1
    );
}

#[test]
fn session_delete_cleans_session_permissions() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let a = session(w, "a");
        let b = session(w, "b");
        for (scope, id) in [("session", a), ("session", b), ("global", 0)] {
            w.execute(
                "INSERT INTO permissions(scope, scope_id, action, mode) VALUES (?1, ?2, 'send', 'ask')",
                params![scope, id],
            )?;
        }
        w.execute("DELETE FROM sessions WHERE id = ?1", params![a])?;
        Ok(())
    })
    .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM permissions"),
        2,
        "session-level a deleted, the rest intact"
    );
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM permissions WHERE scope = 'global'"
        ),
        1
    );
}

#[test]
fn persona_delete_cleans_persona_permissions() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let p = persona(w, "Work");
        let q = persona(w, "Personal");
        for id in [p, q] {
            w.execute(
                "INSERT INTO permissions(scope, scope_id, action, mode) VALUES ('persona', ?1, 'pay', 'deny')",
                params![id],
            )?;
        }
        w.execute("DELETE FROM personas WHERE id = ?1", params![p])?;
        Ok(())
    })
    .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM permissions WHERE scope = 'persona'"
        ),
        1
    );
}

#[test]
fn finding_ref_nulled_when_action_deleted() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let a = action(w, None, None);
        w.execute(
            "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict) VALUES (1, 'dangerous_cmd', 'high', 'actions_log', CAST(?1 AS TEXT), 'asked')",
            params![a],
        )?;
        w.execute("DELETE FROM actions_log WHERE id = ?1", params![a])?;
        Ok(())
    })
    .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM observer_findings WHERE ref_id IS NULL"
        ),
        1
    );
}

#[test]
fn actions_log_survives_persona_delete() {
    let (_d, _p, db) = fresh();
    db.write(|w| {
        let p = persona(w, "Work");
        action(w, None, Some(p));
        w.execute("DELETE FROM personas WHERE id = ?1", params![p])?;
        Ok(())
    })
    .expect("scenario");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM actions_log"), 1);
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM actions_log WHERE persona_id IS NULL"
        ),
        1
    );
}

#[test]
fn messages_streaming_become_interrupted_on_start() {
    let (_d, path, db) = fresh();
    db.write(|w| {
        let s = session(w, "s");
        w.execute(
            "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES (?1, 'agent', 'part', 'streaming', 1)",
            params![s],
        )?;
        Ok(())
    })
    .expect("row");
    drop(db);
    let d = tokio::runtime::Runtime::new().expect("rt").block_on(async {
        FakeAgentd::start(Config::temp().no_ticker().db(path.clone()))
            .await
            .expect("start")
    });
    let status: String = d
        .db()
        .reader()
        .expect("reader")
        .query_row("SELECT status FROM messages", [], |r| r.get(0))
        .unwrap_or_default();
    assert_eq!(status, "interrupted");
    let body: String = d
        .db()
        .reader()
        .expect("reader")
        .query_row("SELECT body_md FROM messages", [], |r| r.get(0))
        .unwrap_or_default();
    assert_eq!(body, "part", "the accumulated text is saved");
}
