//! A5-03: event sources: agent reply, observer findings, provider failures.

mod common;

use std::time::Duration;

use common::{Env, list};
use core_client::api::FrameKind;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn events_agent_reply_in_inactive_session() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    let snap = ws.subscribe("events");
    assert_eq!(snap["items"], json!([]), "snapshot is empty: {snap}");
    let (st, v) = e.say(s, "c1", "hello").await;
    assert!(st == 201 || st == 202, "{v}");
    e.wait_agent_done(s, 1).await;
    let frames = ws.until(Duration::from_secs(10), |f| {
        f.topic == "events" && f.kind == FrameKind::Event
    });
    let ev = &frames.last().unwrap().data;
    assert_eq!(
        (ev["kind"].as_str(), ev["sub"].as_str()),
        (Some("ai"), Some("agent"))
    );
    assert_eq!(ev["session_id"], s);
    assert_eq!(ev["unread"], true);
    let v = list(&e, &format!("?session_id={s}")).await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["items"][0]["id"], ev["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_agent_reply_active_session_no_event() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "hello").await;
    e.wait_agent_done(s, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        list(&e, "").await["items"],
        json!([]),
        "session is open, so no event"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_require_token() {
    let e = Env::start().await;
    let bad = common::raw(&e.sock, "agt_nope", "GET", "/events", None, b"").await;
    assert_eq!(bad.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_observer_finding_emits_ai_watch() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.admin
        .put("/permissions/global/0/shell", json!({"mode": "allow"}))
        .await;
    e.say(s, "c1", "#scn:action_danger clear").await;
    e.wait_agent_done(s, 2).await;
    let v = list(&e, "?kind=ai&sub=watch").await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "one finding, one event: {v}");
    assert_eq!(items[0]["level"], "error", "verdict blocked: {v}");
    assert_eq!(items[0]["source"], "agentd");
    assert!(!items[0]["text"].as_str().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn events_provider_error_emits_ai_event() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:rate_limited hello").await;
    e.wait_agent_done(s, 1).await;
    for _ in 0..100 {
        if !list(&e, "?kind=ai&sub=local").await["items"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let v = list(&e, "?kind=ai&sub=local").await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{v}");
    assert_eq!(items[0]["level"], "error");
    assert_eq!(items[0]["session_id"], s);
    assert!(
        !items[0]["text"].as_str().unwrap().contains("secret"),
        "event text without internal details"
    );
}
