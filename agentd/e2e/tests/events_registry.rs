//! A5-03: subtype registry, computed tab, event-to-session link.

mod common;

use std::time::Duration;

use common::{Env, list, mixed, seed};
use core_client::api::FrameKind;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn events_subtypes_default_11() {
    let e = Env::start().await;
    let (st, v) = e.read.get("/events/subtypes").await;
    assert_eq!(st, 200, "{v}");
    let mut pairs: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            assert!(!s["label"].as_str().unwrap_or("").is_empty(), "{s}");
            assert!(s["icon"].is_string(), "{s}");
            assert_eq!(s["enabled"], true);
            format!(
                "{}.{}",
                s["kind"].as_str().unwrap(),
                s["sub"].as_str().unwrap()
            )
        })
        .collect();
    pairs.sort();
    let mut want: Vec<String> = [
        "ai.agent",
        "ai.watch",
        "ai.claude",
        "ai.local",
        "task.sched",
        "task.deleg",
        "task.done",
        "msg.sys",
        "msg.wa",
        "msg.tg",
        "msg.mail",
    ]
    .map(String::from)
    .to_vec();
    want.sort();
    assert_eq!(pairs, want);
    let wa = v
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["sub"] == "wa")
        .unwrap();
    assert!(
        wa["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("unofficial"),
        "the msg.wa label carries the unofficial-channel mark (Q9): {wa}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_unknown_sub_passthrough() {
    let e = Env::start_with(seed(vec![("msg", "signal", "info", None, 1, 1)])).await;
    let v = list(&e, "").await;
    assert_eq!(v["items"][0]["sub"], "signal");
    assert_eq!(v["counts"]["by_sub"]["msg.signal"], 1);
    assert_eq!(
        list(&e, "?sub=signal").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_tab_computed_default_chat() {
    let mut e = Env::start_with(seed(mixed())).await;
    for it in list(&e, "").await["items"].as_array().unwrap() {
        assert_eq!(it["tab"], "chat", "{it}");
    }
    let cols: Vec<String> = e
        .inspect(|c| {
            c.prepare("SELECT name FROM pragma_table_info('events')")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert!(
        !cols.contains(&"tab".to_string()),
        "tab is not stored in the DB: {cols:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_read_marks_session_events() {
    let e = Env::start_with(seed(mixed())).await;
    let mut ws = e.ws();
    ws.subscribe("events");
    assert_eq!(e.chat.post("/sessions/1/read", json!({})).await.0, 204);
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "events" && f.kind == FrameKind::Delta
    });
    assert_eq!(f.last().unwrap().data["read"].as_array().unwrap().len(), 2);
    let v = list(&e, "?session_id=1").await;
    assert_eq!(v["counts"]["total"], 0);
    let other = list(&e, "?kind=msg&sub=tg").await;
    assert_eq!(other["counts"]["total"], 1, "system event is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_keeps_system_events() {
    let e = Env::start_with(seed(mixed())).await;
    assert_eq!(e.chat.delete("/sessions/1").await.0, 204);
    let v = list(&e, "").await;
    assert_eq!(v["items"].as_array().unwrap().len(), 5, "events remain");
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["session_id"] != 1),
        "{v}"
    );
}
