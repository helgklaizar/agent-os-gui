//! WS topic `events`: snapshot, "created" frame, resubscription, events and sessions.

mod chat_common;
mod events_common;

use events_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn events_ws_snapshot_lists_existing_events_with_counts() {
    let r = bare().await;
    let a = r.add("ai", "agent", "a", None);
    let b = r.add("msg", "wa", "b", None);
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let snap = w.snapshot();
    let ids: Vec<i64> = snap["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e["id"].as_i64())
        .collect();
    assert_eq!(ids, vec![b, a], "snapshot = same as GET /events");
    assert_eq!(snap["counts"]["total"], 2);
    assert_eq!(snap["items"][0]["tab"], "chat");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_ws_created_frame_is_event_body() {
    let r = bare().await;
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let s = r.session().await;
    let id = r.add("task", "sched", "scheduled", Some(s));
    let frame = w
        .until(5, |w| w.all().into_iter().find(|f| f["type"] == "event"))
        .await
        .unwrap_or_else(|| panic!("no event frame: {:?}", w.all()));
    let e = &frame["data"];
    assert_eq!(e["id"], id);
    assert_eq!(
        (e["kind"].clone(), e["sub"].clone()),
        (json!("task"), json!("sched"))
    );
    assert_eq!(e["session_id"], s);
    assert_eq!(e["unread"], true);
    assert_eq!(e["tab"], "chat");
    let listed = r.events("").await;
    assert_eq!(
        listed["items"][0], *e,
        "the frame and GET /events describe the event identically"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_ws_resubscribe_gets_fresh_snapshot() {
    let r = bare().await;
    let w1 = watch(&r.d, &r.admin, "events").await;
    assert!(
        w1.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    r.d.drop_ws();
    let id = r.add("msg", "tg", "while offline", None);
    r.d.restore();
    let w2 = watch(&r.d, &r.admin, "events").await;
    assert!(
        w2.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let first = w2.snapshot()["items"][0]["id"].clone();
    assert_eq!(first, id, "a missed event arrives with the snapshot");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_survive_session_delete_with_null_session() {
    let r = bare().await;
    let s = r.session().await;
    let id = r.add("ai", "agent", "reply", Some(s));
    let resp = r.call("DELETE", &format!("/v1/sessions/{s}"), None).await;
    assert_eq!(resp.status, 204);
    let v = r.events("").await;
    assert_eq!(v["items"][0]["id"], id);
    assert_eq!(v["items"][0]["session_id"], serde_json::Value::Null);
    assert_eq!(unread_total(&v), 1, "the event stayed unread");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_ws_duplicate_event_does_not_create_frame() {
    let r = bare().await;
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let e = fake_agentd::events_db::NewEvent {
        kind: "msg",
        sub: "sys",
        text: "x",
        dedup_key: Some("cpu:1"),
        ts: 1,
        ..Default::default()
    };
    assert!(matches!(r.d.add_event(&e), Ok(Some(_))));
    assert!(
        matches!(r.d.add_event(&e), Ok(None)),
        "a repeated dedup_key is not an event"
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let created = w.all().into_iter().filter(|f| f["type"] == "event").count();
    assert_eq!(created, 1, "exactly one frame");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_ws_snapshot_carries_last_200_and_counts_of_all() {
    let r = bare().await;
    for i in 0..205 {
        r.add("msg", "sys", &format!("e{i}"), None);
    }
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let snap = w.snapshot();
    assert_eq!(
        snap["items"].as_array().map(Vec::len),
        Some(200),
        "the latest 200"
    );
    assert_eq!(
        snap["counts"]["total"], 205,
        "counters cover all events, not the page"
    );
}
