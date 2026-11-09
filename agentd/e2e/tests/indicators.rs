//! A5-04: indicators: settings with `If-Match`, state transitions and event deduplication, the `bar.indicators` topic.

mod common;

use std::time::Duration;

use common::Env;
use core_client::api::FrameKind;
use serde_json::{Value, json};

const Q2: [&str; 8] = [
    "cpu", "ram", "disk", "temp", "net", "ai_usage", "unread", "agentd",
];

async fn frame(e: &Env, id: &str, state: &str) {
    let (st, v) = e
        .admin
        .post(
            "/_test/indicator",
            json!({"id": id, "state": state, "value": 1, "text": "frame"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

async fn events(e: &Env) -> Vec<Value> {
    let (st, v) = e.read.get("/events?kind=msg&sub=sys").await;
    assert_eq!(st, 200, "{v}");
    let mut items = v["items"].as_array().unwrap().clone();
    items.reverse();
    items
}

fn levels(items: &[Value]) -> Vec<&str> {
    items.iter().map(|i| i["level"].as_str().unwrap()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_defaults_q2() {
    let e = Env::start().await;
    let (st, v) = e.read.get("/settings/indicators").await;
    assert_eq!(st, 200, "{v}");
    let items = v["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, Q2, "eight Q2 indicators in order: {v}");
    assert!(items.iter().all(|i| i["enabled"] == true));
    let pos: Vec<u64> = items
        .iter()
        .map(|i| i["position"].as_u64().unwrap())
        .collect();
    assert_eq!(pos, (0..8).collect::<Vec<u64>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn indicators_patch_if_match() {
    let e = Env::start().await;
    let (_, page) = e.read.get("/settings/indicators").await;
    let rev = page["rev"].as_i64().unwrap_or(0);
    let items = json!({"items": [{"id": "cpu", "enabled": false, "position": 0}]});
    assert_eq!(
        e.admin.patch("/settings/indicators", items.clone()).await.0,
        428
    );
    let (st, _) = e
        .admin
        .patch_if_match("/settings/indicators", items.clone(), rev + 7)
        .await;
    assert_eq!(st, 412);
    let (st, v) = e
        .admin
        .patch_if_match("/settings/indicators", items.clone(), rev)
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["rev"], rev + 1);
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let (st, _) = e
        .admin
        .patch_if_match("/settings/indicators", items, rev)
        .await;
    assert_eq!(st, 412, "stale rev after a write");
    let bad = json!({"items": [{"id": "", "enabled": "yes", "position": -1}]});
    let (st, _) = e
        .admin
        .patch_if_match("/settings/indicators", bad, rev + 1)
        .await;
    assert_eq!(st, 422);
    let (st, _) = e
        .chat
        .patch_if_match("/settings/indicators", json!({"items": []}), rev + 1)
        .await;
    assert_eq!(st, 403, "admin required");
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_red_100_frames_one_event() {
    let e = Env::start().await;
    for _ in 0..100 {
        frame(&e, "probe1", "bad").await;
    }
    let ev = events(&e).await;
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["level"], "error");
    assert_eq!(ev[0]["source"], "agentd");
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_red_read_red_single_event() {
    let e = Env::start().await;
    frame(&e, "probe1", "bad").await;
    assert_eq!(e.chat.post("/events/read", json!({})).await.1["marked"], 1);
    frame(&e, "probe1", "bad").await;
    frame(&e, "probe1", "bad").await;
    assert_eq!(
        events(&e).await.len(),
        1,
        "read state creates no new events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_red_to_yellow_event() {
    let e = Env::start().await;
    frame(&e, "probe1", "bad").await;
    frame(&e, "probe1", "warn").await;
    assert_eq!(levels(&events(&e).await), ["error", "warn"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_return_to_ok_and_red_again() {
    let e = Env::start().await;
    frame(&e, "probe1", "bad").await;
    frame(&e, "probe1", "ok").await;
    frame(&e, "probe1", "ok").await;
    assert_eq!(levels(&events(&e).await), ["error", "ok"]);
    frame(&e, "probe1", "bad").await;
    assert_eq!(levels(&events(&e).await), ["error", "ok", "error"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_first_ok_is_silent_and_indicators_are_independent() {
    let e = Env::start().await;
    frame(&e, "probe1", "ok").await;
    frame(&e, "probe2", "nodata").await;
    assert!(
        events(&e).await.is_empty(),
        "initialization without an event"
    );
    frame(&e, "probe1", "bad").await;
    frame(&e, "probe2", "bad").await;
    assert_eq!(
        events(&e).await.len(),
        2,
        "each indicator has its own transition count"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_state_survives_restart() {
    let mut e = Env::start().await;
    frame(&e, "probe1", "bad").await;
    frame(&e, "probe1", "warn").await;
    e.crash_and_restart();
    frame(&e, "probe1", "warn").await;
    assert_eq!(
        events(&e).await.len(),
        2,
        "the same frame after a restart gives no event"
    );
    let row: (String, i64, Option<i64>) = e
        .inspect(|c| {
            c.query_row(
                "SELECT last_state, transition_seq, last_change_at FROM indicators WHERE id = 'probe1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!((row.0.as_str(), row.1), ("warn", 2));
    assert!(row.2.is_some());
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.indicators");
    let cpu = snap["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "probe1")
        .cloned()
        .expect("cpu in snapshot");
    assert_eq!(cpu["state"], "warn");
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_topic_delta_shape() {
    let e = Env::start().await;
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.indicators");
    assert!(
        snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["id"] != "probe3"),
        "disk not observed yet: {snap}"
    );
    frame(&e, "probe3", "warn").await;
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "bar.indicators" && f.kind == FrameKind::Delta
    });
    let d = &f.last().unwrap().data;
    assert_eq!(
        (d["id"].as_str(), d["state"].as_str(), d["text"].as_str()),
        (Some("probe3"), Some("warn"), Some("frame"))
    );
    assert_eq!(d["value"], 1);
    assert!(d["ts"].as_i64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_transition_event_frame_in_events_topic() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("events");
    frame(&e, "net", "bad").await;
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "events" && f.kind == FrameKind::Event
    });
    let ev = &f.last().unwrap().data;
    assert_eq!(
        (ev["kind"].as_str(), ev["sub"].as_str()),
        (Some("msg"), Some("sys"))
    );
    assert_eq!(ev["level"], "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_rejects_bad_frames_and_foreign_tokens() {
    let e = Env::start().await;
    let body = |state: &str| json!({"id": "probe1", "state": state});
    assert_eq!(e.admin.post("/_test/indicator", body("red")).await.0, 422);
    assert_eq!(
        e.admin
            .post("/_test/indicator", json!({"state": "ok"}))
            .await
            .0,
        422
    );
    assert_eq!(e.chat.post("/_test/indicator", body("ok")).await.0, 403);
    assert!(events(&e).await.is_empty());
}
