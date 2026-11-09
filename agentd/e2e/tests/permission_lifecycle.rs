//! A4-04: lifecycle of open requests: deleting a session and restarting the core close them with a denial.

mod common;

use common::{Env, ask_delete, db_decision, decide, session};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_denies_open_requests() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (st, _) = e.chat.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 204);
    let (st, _) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(st, 404, "card unavailable");
    let (st, _) = decide(&e.admin, id, json!({"decision": "allow"})).await;
    assert_eq!(st, 404);
    let row: (Option<String>, Option<String>, Option<i64>) = e
        .inspect(move |c| {
            c.query_row(
                "SELECT decision, result, session_id FROM actions_log WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!(
        row,
        (Some("deny".into()), Some("session_deleted".into()), None)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_requests_do_not_survive_restart() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    e.crash_and_restart();
    let (st, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!((st, cur["status"].as_str()), (200, Some("denied")), "{cur}");
    assert_eq!(
        db_decision(&mut e, id).await,
        (Some("deny".into()), Some("core_restart".into()))
    );
}
