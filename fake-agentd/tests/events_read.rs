//! Marking as read: `POST /events/{id}/read`, `POST /events/read`, and `events` topic frames.

mod chat_common;
mod events_common;

use events_common::*;
use serde_json::{Value, json};

fn read_frames(w: &Watch) -> Vec<Value> {
    w.all()
        .into_iter()
        .filter(|f| f["type"] == "delta" && f["data"].get("read").is_some())
        .map(|f| f["data"]["read"].clone())
        .collect()
}

async fn subscribed(r: &Rig) -> Watch {
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some(),
        "events snapshot"
    );
    w
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_one_read_204_and_frame() {
    let r = bare().await;
    let w = subscribed(&r).await;
    let a = r.add("ai", "agent", "a", None);
    let b = r.add("ai", "agent", "b", None);
    let resp = r.call("POST", &format!("/v1/events/{a}/read"), None).await;
    assert_eq!(resp.status, 204);
    assert_eq!(r.ids("unread=true").await, vec![b]);
    assert!(
        w.until(5, |w| (!read_frames(w).is_empty()).then_some(()))
            .await
            .is_some(),
        "the \"read\" frame"
    );
    assert_eq!(read_frames(&w), vec![json!([a])]);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_one_read_is_idempotent_without_second_frame() {
    let r = bare().await;
    let w = subscribed(&r).await;
    let a = r.add("ai", "agent", "a", None);
    for _ in 0..2 {
        let resp = r.call("POST", &format!("/v1/events/{a}/read"), None).await;
        assert_eq!(resp.status, 204);
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        read_frames(&w).len(),
        1,
        "a repeat changes nothing and sends no frame"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_one_read_unknown_id_404() {
    let r = bare().await;
    let resp = r.call("POST", "/v1/events/999/read", None).await;
    assert_eq!(resp.status, 404);
    assert_eq!(code(&resp), "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_all_read_by_filter() {
    let r = bare().await;
    let s = r.session().await;
    let w = subscribed(&r).await;
    let a = r.add("ai", "agent", "a", Some(s));
    let b = r.add("ai", "watch", "b", None);
    let c = r.add("task", "done", "c", Some(s));
    let d = r.add("msg", "wa", "d", None);
    let v = r.read_all(json!({"kind": "ai"})).await;
    assert_eq!(v["marked"], 2);
    assert_eq!(r.ids("unread=true").await, vec![d, c]);
    let v = r.read_all(json!({"sub": "wa"})).await;
    assert_eq!(v["marked"], 1);
    let v = r.read_all(json!({"session_id": s})).await;
    assert_eq!(v["marked"], 1, "only c is left");
    assert_eq!(r.ids("unread=true").await, Vec::<i64>::new());
    assert!(
        w.until(5, |w| (read_frames(w).len() >= 3).then_some(()))
            .await
            .is_some(),
        "one frame per call"
    );
    let mut got: Vec<Vec<i64>> = read_frames(&w)
        .iter()
        .map(|v| {
            v.as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_i64)
                .collect()
        })
        .collect();
    for g in &mut got {
        g.sort_unstable();
    }
    assert_eq!(got, vec![vec![a, b], vec![d], vec![c]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_all_read_empty_body_marks_everything_unread() {
    let r = bare().await;
    for i in 0..3 {
        r.add("msg", "sys", &format!("e{i}"), None);
    }
    let resp = r.call("POST", "/v1/events/read", None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["marked"], 3);
    let v = r.events("").await;
    assert_eq!(unread_total(&v), 0);
    let again = r.read_all(empty_body()).await;
    assert_eq!(again["marked"], 0, "nothing unread");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_all_read_by_ids_ignores_unknown() {
    let r = bare().await;
    let a = r.add("ai", "agent", "a", None);
    r.add("ai", "agent", "b", None);
    let v = r.read_all(json!({"ids": [a, 9999]})).await;
    assert_eq!(v["marked"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_all_read_rejects_bad_body() {
    let r = bare().await;
    for body in [
        json!({"kind": "bogus"}),
        json!({"sub": 5}),
        json!({"session_id": "x"}),
        json!({"ids": "all"}),
        json!({"ids": [1, "x"]}),
        json!([1, 2]),
    ] {
        let resp = r.call("POST", "/v1/events/read", Some(body.clone())).await;
        assert!(
            matches!(resp.status, 400 | 422),
            "{body}: {} {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    let ev = r.add("ai", "agent", "a", None);
    assert_eq!(
        r.ids("unread=true").await,
        vec![ev],
        "a rejected call marks nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_read_routes_need_chat_scope() {
    let r = bare().await;
    let a = r.add("ai", "agent", "a", None);
    let reader = r.token(fake_agentd::Scope::Read, fake_agentd::Kind::External);
    let one = r
        .call_as(&reader, "POST", &format!("/v1/events/{a}/read"), None)
        .await;
    assert_eq!((one.status, code(&one)), (403, "insufficient_scope".into()));
    let all = r.call_as(&reader, "POST", "/v1/events/read", None).await;
    assert_eq!((all.status, code(&all)), (403, "insufficient_scope".into()));
    assert_eq!(
        r.ids("unread=true").await,
        vec![a],
        "a foreign scope changes nothing"
    );
}
