//! A5-03: event feed (`/events`): filters, counters, pages, mark as read.

mod common;

use std::time::Duration;

use common::{Env, fields, ids, list, mixed, seed};
use core_client::api::FrameKind;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn events_filters_and_counts() {
    let e = Env::start_with(seed(mixed())).await;
    let all = list(&e, "").await;
    assert_eq!(all["items"].as_array().unwrap().len(), 5, "{all}");
    assert_eq!(all["counts"]["total"], 4, "4 unread: {all}");
    assert_eq!(all["counts"]["by_kind"], json!({"ai": 2, "msg": 2}));
    assert_eq!(
        all["counts"]["by_sub"],
        json!({"ai.agent": 1, "ai.watch": 1, "msg.tg": 1, "msg.wa": 1})
    );
    let ai = list(&e, "?kind=ai").await;
    assert_eq!(ai["items"].as_array().unwrap().len(), 2);
    assert_eq!(ai["counts"]["total"], 2, "counters per selection: {ai}");
    let sub = list(&e, "?kind=msg&sub=wa").await;
    assert_eq!(sub["items"].as_array().unwrap().len(), 1);
    assert_eq!(sub["items"][0]["level"], "error");
    let unread = list(&e, "?unread=true").await;
    assert_eq!(unread["items"].as_array().unwrap().len(), 4);
    let read = list(&e, "?unread=false").await;
    assert_eq!(read["items"].as_array().unwrap().len(), 1);
    let s1 = list(&e, "?session_id=1").await;
    assert_eq!(s1["items"].as_array().unwrap().len(), 2);
    assert_eq!(e.read.get("/events?kind=zzz").await.0, 422);
    assert_eq!(e.read.get("/events?limit=0").await.0, 422);
    assert_eq!(e.read.get("/events?before=x").await.0, 422);
    assert_eq!(e.read.get("/events?unread=maybe").await.0, 422);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_item_shape() {
    let e = Env::start_with(seed(vec![("msg", "tg", "warn", Some(1), 1, 77)])).await;
    let item = list(&e, "").await["items"][0].clone();
    assert_eq!(item["kind"], "msg");
    assert_eq!(item["sub"], "tg");
    assert_eq!(item["level"], "warn");
    assert_eq!(item["text"], "t");
    assert_eq!(item["session_id"], 1);
    assert_eq!(item["unread"], true);
    assert_eq!(item["ts"], 77);
    assert_eq!(item["source"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_pagination_by_id() {
    let e = Env::start_with(seed(vec![
        ("ai", "agent", "info", None, 1, 5),
        ("ai", "agent", "info", None, 1, 5),
        ("ai", "agent", "info", None, 1, 5),
    ]))
    .await;
    let p1 = list(&e, "?limit=2").await;
    let first = ids(&p1);
    assert_eq!(first.len(), 2);
    assert!(first[0] > first[1], "newest first: {first:?}");
    assert_eq!(
        p1["counts"]["total"], 3,
        "counter does not depend on the page"
    );
    let p2 = list(&e, &format!("?limit=2&before={}", first[1])).await;
    let rest = ids(&p2);
    assert_eq!(rest.len(), 1);
    assert!(rest[0] < first[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_read_all_by_filter() {
    let e = Env::start_with(seed(mixed())).await;
    let mut ws = e.ws();
    ws.subscribe("events");
    let (st, v) = e.chat.post("/events/read", json!({"session_id": 1})).await;
    assert_eq!((st, &v["marked"]), (200, &json!(2)), "{v}");
    let frames = ws.until(Duration::from_secs(10), |f| {
        f.topic == "events" && f.kind == FrameKind::Delta
    });
    let mut read: Vec<i64> = frames.last().unwrap().data["read"]
        .as_array()
        .expect("delta {read}")
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    read.sort_unstable();
    assert_eq!(read.len(), 2);
    assert_eq!(list(&e, "").await["counts"]["total"], 2);
    let (_, v) = e.chat.post("/events/read", json!({"kind": "msg"})).await;
    assert_eq!(v["marked"], 1);
    let (_, v) = e.chat.post("/events/read", json!({})).await;
    assert_eq!(v["marked"], 1, "empty body marks all unread: {v}");
    let (_, v) = e.chat.post("/events/read", json!({})).await;
    assert_eq!(v["marked"], 0);
    assert_eq!(list(&e, "").await["counts"]["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_read_by_ids_and_validation() {
    let e = Env::start_with(seed(mixed())).await;
    let all = ids(&list(&e, "").await);
    let (st, v) = e
        .chat
        .post("/events/read", json!({"ids": [all[0], all[1], 9999]}))
        .await;
    assert_eq!((st, &v["marked"]), (200, &json!(2)), "{v}");
    let (st, v) = e.chat.post("/events/read", json!({"kind": "zzz"})).await;
    assert_eq!(st, 422);
    assert!(fields(&v).contains(&"kind".to_string()), "{v}");
    assert_eq!(e.chat.post("/events/read", json!({"x": 1})).await.0, 422);
    assert_eq!(
        e.chat.post("/events/read", json!({"ids": "a"})).await.0,
        422
    );
    assert_eq!(
        e.read.post("/events/read", json!({})).await.0,
        403,
        "chat required"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_read_one() {
    let e = Env::start_with(seed(mixed())).await;
    let id = ids(&list(&e, "?unread=true").await)[0];
    let mut ws = e.ws();
    ws.subscribe("events");
    assert_eq!(
        e.chat
            .post(&format!("/events/{id}/read"), json!({}))
            .await
            .0,
        204
    );
    let f = ws.until(Duration::from_secs(10), |f| f.kind == FrameKind::Delta);
    assert_eq!(f.last().unwrap().data["read"], json!([id]));
    assert_eq!(list(&e, "").await["counts"]["total"], 3);
    assert_eq!(
        e.chat
            .post(&format!("/events/{id}/read"), json!({}))
            .await
            .0,
        204,
        "replay is idempotent"
    );
    assert_eq!(e.chat.post("/events/9999/read", json!({})).await.0, 404);
    assert_eq!(e.chat.post("/events/x/read", json!({})).await.0, 400);
    assert_eq!(
        e.read
            .post(&format!("/events/{id}/read"), json!({}))
            .await
            .0,
        403
    );
}
