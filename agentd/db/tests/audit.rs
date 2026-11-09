//! A4-05 (DB): action log: secret masking, irreversibility, surviving session and persona deletion, CHECK.

use std::sync::Arc;

use agentd_core::{Actor, Audit, AuditStore, Decision, ManualClock, NewAction};
use agentd_db::{Db, DbConfig, PermRepo};
use serde_json::json;

async fn setup() -> (tempfile::TempDir, Arc<Db>, Audit, Arc<PermRepo>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("sessions.db"))).unwrap());
    db.write(|tx| {
        tx.execute_batch(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1);
             INSERT INTO personas(id, name, color) VALUES (7,'p','#fff');",
        )
    })
    .await
    .unwrap();
    let repo = Arc::new(PermRepo::new(db.clone()));
    let audit = Audit::new(repo.clone(), Arc::new(ManualClock::new(1_800_000_000)));
    (dir, db, audit, repo)
}

fn entry(action: &str, args: serde_json::Value) -> NewAction {
    NewAction {
        actor: Actor::Agent,
        session_id: Some(1),
        persona_id: Some(7),
        action: action.into(),
        target: Some("t".into()),
        args: Some(args),
        decision: Some(Decision::Allow),
        result: Some("ok".into()),
    }
}

async fn one(
    db: &Db,
    id: i64,
) -> (
    Option<i64>,
    Option<i64>,
    Option<String>,
    i64,
    Option<String>,
) {
    db.read(move |c| {
        c.query_row(
            "SELECT session_id, persona_id, args_json, reversible, undo_json FROM actions_log WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn secrets_masked_in_log_args() {
    let (_d, db, audit, _r) = setup().await;
    let id = audit
        .record(entry(
            "shell",
            json!({"cmd": "curl", "token": "tok-VALUE-1", "headers": {"Authorization": "Bearer abcdefghijkl"},
                   "key": "kv-VALUE-2"}),
        ))
        .await
        .unwrap();
    let (_, _, args, ..) = one(&db, id).await;
    let args = args.unwrap();
    for leak in ["tok-VALUE-1", "abcdefghijkl", "kv-VALUE-2"] {
        assert!(!args.contains(leak), "{leak} in {args}");
    }
    assert!(
        args.contains("\"cmd\":\"curl\"") && args.contains("***"),
        "{args}"
    );
}

#[tokio::test]
async fn audit_irreversible_flags() {
    let (_d, db, audit, _r) = setup().await;
    for a in ["send", "pay"] {
        let id = audit.record(entry(a, json!({"to": "x"}))).await.unwrap();
        let (.., reversible, undo) = one(&db, id).await;
        assert_eq!((reversible, undo), (0, None), "{a}");
    }
}

#[tokio::test]
async fn actions_log_survives_session_delete() {
    let (_d, db, audit, _r) = setup().await;
    let id = audit
        .record(entry("send", json!({"to": "x"})))
        .await
        .unwrap();
    db.write(|tx| {
        tx.execute("DELETE FROM sessions WHERE id = 1", [])
            .map(drop)
    })
    .await
    .unwrap();
    let (session, persona, args, ..) = one(&db, id).await;
    assert_eq!((session, persona, args), (None, Some(7), None));
}

#[tokio::test]
async fn actions_log_survives_persona_delete() {
    let (_d, db, audit, _r) = setup().await;
    let id = audit
        .record(entry("send", json!({"to": "x"})))
        .await
        .unwrap();
    db.write(|tx| {
        tx.execute("DELETE FROM personas WHERE id = 7", [])
            .map(drop)
    })
    .await
    .unwrap();
    let (session, persona, args, ..) = one(&db, id).await;
    assert_eq!((session, persona), (Some(1), None));
    assert!(args.is_some(), "args live while the session lives");
}

#[tokio::test]
async fn actions_log_decision_check() {
    let (_d, db, _a, _r) = setup().await;
    let bad = db
        .write(|tx| {
            tx.execute(
                "INSERT INTO actions_log(ts, actor, action, decision) VALUES (1,'agent','send','maybe')",
                [],
            )
            .map(drop)
        })
        .await;
    assert!(bad.is_err(), "decision outside the set was accepted");
    let bad = db
        .write(|tx| {
            tx.execute(
                "INSERT INTO actions_log(ts, actor, action) VALUES (1,'robot','send')",
                [],
            )
            .map(drop)
        })
        .await;
    assert!(bad.is_err(), "actor outside the set was accepted");
}

#[tokio::test]
async fn observer_verdict_check() {
    let (_d, db, _a, _r) = setup().await;
    let bad = db
        .write(|tx| {
            tx.execute(
                "INSERT INTO observer_findings(ts, kind, severity, verdict) VALUES (1,'injection','low','maybe')",
                [],
            )
            .map(drop)
        })
        .await;
    assert!(bad.is_err(), "verdict outside the set was accepted");
}

#[tokio::test]
async fn decide_action_is_first_wins() {
    let (_d, _db, audit, repo) = setup().await;
    let mut e = entry("send", json!({}));
    e.decision = None;
    e.result = None;
    let id = audit.record(e).await.unwrap();
    assert_eq!(repo.pending_actions(None).await.unwrap().len(), 1);
    assert!(
        repo.decide_action(id, Decision::AskApproved, None, None)
            .await
            .unwrap()
    );
    assert!(
        !repo
            .decide_action(id, Decision::AskDenied, None, None)
            .await
            .unwrap()
    );
    let row = repo.get_action(id).await.unwrap().unwrap();
    assert_eq!(row.decision, Some(Decision::AskApproved));
    assert!(repo.pending_actions(None).await.unwrap().is_empty());
}
