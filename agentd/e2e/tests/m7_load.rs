//! A7-15 (M7): load checks on the real core: 5000 messages in a session, a 100k-character stream, 100 concurrent WS subscriptions.
//! Thresholds are working assumptions of A7 (not set by the spec); numbers are printed (`--nocapture`) and go into the report.

mod common;

use std::time::{Duration, Instant};

use common::{Env, is_kind};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn m7_load_5000_messages_in_session() {
    let e = Env::start_with(|c| {
        c.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('load', 1, 1)",
            [],
        )?;
        for i in 0..5000 {
            c.execute(
                "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES (1, ?1, ?2, 'done', ?3)",
                (if i % 2 == 0 { "user" } else { "agent" }, format!("message {i}"), i),
            )?;
        }
        Ok(())
    })
    .await;
    // History page: the last 200.
    let t = Instant::now();
    let (st, page) = e.read.get("/sessions/1/messages?limit=200").await;
    let page_ms = t.elapsed().as_millis();
    assert_eq!(st, 200);
    assert_eq!(page.as_array().unwrap().len(), 200);
    // Walk the whole history backwards in pages of 200 (25 pages); nothing is lost or repeated.
    let t = Instant::now();
    let mut seen = 0usize;
    let mut before: Option<i64> = None;
    loop {
        let path = before.map_or_else(
            || "/sessions/1/messages?limit=200".to_string(),
            |b| format!("/sessions/1/messages?limit=200&before={b}"),
        );
        let (st, p) = e.read.get(&path).await;
        assert_eq!(st, 200);
        let items = p.as_array().unwrap();
        if items.is_empty() {
            break;
        }
        seen += items.len();
        before = items.iter().filter_map(|m| m["id"].as_i64()).min();
    }
    let walk_ms = t.elapsed().as_millis();
    assert_eq!(seen, 5000, "every message received exactly once");
    // Session subscription: a snapshot with the last page.
    let mut ws = e.ws();
    let t = Instant::now();
    let snap = ws.subscribe("session.1");
    let snap_ms = t.elapsed().as_millis();
    let n = snap["messages"].as_array().unwrap().len();
    assert!(
        n > 0 && n <= 200,
        "snapshot is a page, not the whole history: {n}"
    );
    eprintln!(
        "5000 messages: page of 200 — {page_ms} ms, walk of 25 pages — {walk_ms} ms, subscription snapshot — {snap_ms} ms ({n} messages)"
    );
    assert!(
        page_ms < 1000 && snap_ms < 2000 && walk_ms < 15_000,
        "assumption thresholds"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn m7_load_stream_100k_chars() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let t = Instant::now();
    e.say(s, "c1", "#scn:long").await;
    let frames = ws.until(Duration::from_secs(60), |f| is_kind(f, "message.done"));
    let ms = t.elapsed().as_millis();
    let mut offset = 0u64;
    let mut chars = 0usize;
    for f in frames.iter().filter(|f| is_kind(f, "message.delta")) {
        assert_eq!(
            f.data["offset"].as_u64().unwrap(),
            offset,
            "offset without gaps"
        );
        let text = f.data["text"].as_str().unwrap();
        offset += text.len() as u64;
        chars += text.chars().count();
    }
    let done = frames.last().unwrap();
    assert_eq!(chars, 100_000);
    assert_eq!(
        done.data["final_len"].as_u64().unwrap(),
        offset,
        "final_len = accumulated length"
    );
    assert_eq!(done.data["status"], "done");
    eprintln!(
        "stream of 100000 characters: {ms} ms, {} deltas",
        frames.len()
    );
    assert!(ms < 30_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn m7_load_100_ws_subscriptions() {
    let e = Env::start().await;
    let t = Instant::now();
    let mut clients: Vec<_> = (0..100).map(|_| e.ws()).collect();
    let connect_ms = t.elapsed().as_millis();
    for c in &mut clients {
        c.subscribe("bar.indicators");
    }
    let sub_ms = t.elapsed().as_millis() - connect_ms;
    // One change — a delta for all hundred.
    let (st, v) = e
        .admin
        .post(
            "/_test/indicator",
            json!({"id": "load1", "state": "bad", "value": 1, "text": "frame"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let t = Instant::now();
    for (i, c) in clients.iter_mut().enumerate() {
        let got = c.until(Duration::from_secs(15), |f| {
            f.topic == "bar.indicators" && f.data["id"] == "load1"
        });
        assert!(!got.is_empty(), "client {i} received the delta");
    }
    eprintln!(
        "100 WS: connect {connect_ms} ms, subscribe {sub_ms} ms, delta delivered to all in {} ms",
        t.elapsed().as_millis()
    );
}
