//! A5-12: end-to-end scenario `e2e_events_tasks_gateway` on the real core: an inbound message matching a trigger starts an agent turn,
//! a task runs manually, events from both sources reach the feed, and the counters and read marks agree.

mod common;

use std::time::Duration;

use common::{Env, deleg, finished_runs, mk_task};
use core_client::api::FrameKind;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn e2e_events_tasks_gateway() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let acc = e
        .admin
        .make("/accounts", json!({"type": "tg", "label": "tg"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let (st, v) = e
        .admin
        .post(
            "/triggers",
            json!({"source": "tg", "match": {"contains": "report"}, "session_id": s}),
        )
        .await;
    assert_eq!(st, 201, "{v}");

    let mut ws = e.ws();
    ws.subscribe("events");
    let (st, v) = e
        .admin
        .post(
            "/_test/ingest",
            json!({"account_id": acc, "ext_id": "1", "handle": "@boss", "body": "send the report"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    // Trigger run: the user message and the agent reply in the session.
    for _ in 0..200 {
        if e.messages(s).await.iter().any(|m| m["role"] == "agent") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        e.messages(s).await.iter().any(|m| m["role"] == "agent"),
        "the agent replied to the trigger run"
    );

    let t = mk_task(&e, s, deleg("#scn:chunks do it")).await;
    let id = t["id"].as_i64().unwrap();
    assert_eq!(
        e.chat.post(&format!("/tasks/{id}/run"), json!({})).await.0,
        202
    );
    finished_runs(&e, id, 1).await;

    let frames = ws.until(Duration::from_secs(10), |f| {
        f.topic == "events" && f.kind == FrameKind::Event && f.data["sub"] == "done"
    });
    assert!(
        frames.iter().any(|f| f.data["sub"] == "tg"),
        "the msg.tg event arrived"
    );
    let (_, ev) = e.read.get("/events").await;
    let unread = ev["counts"]["total"].as_i64().unwrap_or(0);
    assert!(unread >= 3, "msg.tg, task.deleg, task.done: {ev}");
    let (st, v) = e.chat.post("/events/read", json!({"kind": "task"})).await;
    assert_eq!((st, v["marked"].as_i64()), (200, Some(2)), "{v}");
    let (_, ev) = e.read.get("/events?kind=task").await;
    assert!(
        ev["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["unread"] == false),
        "{ev}"
    );
}
