//! S4-real: contract mismatches found when running the GUI against `agentd` (api.md sections 4.4 and 5.4).

mod common;

use std::time::Duration;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn session_updated_reaches_session_topic() {
    let e = Env::start().await;
    let s = e.chat.make("/sessions", json!({"name": "s"})).await["id"]
        .as_i64()
        .unwrap();
    let p = e.admin.make("/personas", common::persona("P")).await["id"]
        .as_i64()
        .unwrap();
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let (st, v) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"persona_id": p}))
        .await;
    assert_eq!(st, 200, "{v}");
    let frames = ws.until(Duration::from_secs(5), |f| {
        f.data["kind"] == "session.updated"
    });
    let last = frames.last().unwrap();
    assert_eq!(last.data["session"]["id"], s);
    assert_eq!(last.data["session"]["persona_id"], p);
}
