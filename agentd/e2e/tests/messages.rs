//! A3-07: messages, history, attachments, clearing through the `agentd` binary and `core-client`.

mod common;

use common::{Env, fields};
use serde_json::{Value, json};

fn post(cid: &str, text: &str) -> Value {
    json!({"body_md": text, "client_id": cid})
}

async fn user_ids(e: &Env, s: i64) -> Vec<i64> {
    let (_, all) = e
        .read
        .get(&format!("/sessions/{s}/messages?limit=200"))
        .await;
    let mut ids: Vec<i64> = all
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    ids.reverse();
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn message_post_new_client_id_201() {
    let e = Env::start().await;
    let s = e.session("chat").await;
    let (st, m) = e
        .chat
        .post(&format!("/sessions/{s}/messages"), post("c-1", "hello"))
        .await;
    assert_eq!(st, 201, "{m}");
    let mut keys: Vec<&str> = m.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "author",
            "body_md",
            "client_id",
            "id",
            "meta",
            "role",
            "session_id",
            "status",
            "ts"
        ]
    );
    assert_eq!(
        (
            m["role"].as_str(),
            m["status"].as_str(),
            m["client_id"].as_str()
        ),
        (Some("user"), Some("done"), Some("c-1"))
    );
    assert_eq!(
        (m["body_md"].as_str(), m["session_id"].as_i64()),
        (Some("hello"), Some(s))
    );
    assert!(m["ts"].as_i64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn message_post_idempotent_client_id() {
    let e = Env::start().await;
    let (a, b) = (e.session("a").await, e.session("b").await);
    let (st1, m1) = e
        .chat
        .post(&format!("/sessions/{a}/messages"), post("dup", "text"))
        .await;
    let (st2, m2) = e
        .chat
        .post(&format!("/sessions/{a}/messages"), post("dup", "text"))
        .await;
    assert_eq!((st1, st2), (201, 200), "{m2}");
    assert_eq!(m1["id"], m2["id"]);
    assert_eq!(user_ids(&e, a).await.len(), 1, "no duplicate");
    let (st, err) = e
        .chat
        .post(&format!("/sessions/{a}/messages"), post("dup", "another"))
        .await;
    assert_eq!(
        (st, err["error"]["code"].as_str()),
        (409, Some("client_id_conflict"))
    );
    let (st, other) = e
        .chat
        .post(&format!("/sessions/{b}/messages"), post("dup", "text"))
        .await;
    assert_eq!(st, 201, "the same client_id is allowed in another session");
    assert_ne!(other["id"], m1["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn message_role_user_only_from_client() {
    let e = Env::start().await;
    let s = e.session("x").await;
    for (extra, field) in [
        (json!({"role": "agent"}), "role"),
        (json!({"role": "system"}), "role"),
        (json!({"meta": {"kind": "permission_request"}}), "meta"),
        (json!({"status": "streaming"}), "status"),
        (json!({"author": "Agent · Me"}), "author"),
    ] {
        let mut body = post("c", "x");
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (st, b) = e.chat.post(&format!("/sessions/{s}/messages"), body).await;
        assert_eq!(st, 422, "{extra}: {b}");
        assert!(fields(&b).contains(&field.to_string()), "{b}");
    }
    assert!(user_ids(&e, s).await.is_empty(), "nothing stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn message_meta_from_client_sanitized() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let (st, ok) = e
        .chat
        .post(&format!("/sessions/{s}/messages"), post("c", "x"))
        .await;
    assert_eq!(st, 201, "{ok}");
    assert_eq!(
        ok["meta"],
        Value::Null,
        "only the core creates a permission card"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn message_validation_scope_and_unknown_session() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let path = format!("/sessions/{s}/messages");
    for (body, field) in [
        (json!({"body_md": "x"}), "client_id"),
        (json!({"body_md": "x", "client_id": ""}), "client_id"),
        (json!({"client_id": "c"}), "body_md"),
        (json!({"body_md": "   ", "client_id": "c"}), "body_md"),
        (json!({"body_md": 5, "client_id": "c"}), "body_md"),
        (
            json!({"body_md": "x", "client_id": "c", "attachment_ids": [999]}),
            "attachment_ids",
        ),
    ] {
        let (st, b) = e.chat.post(&path, body.clone()).await;
        assert_eq!(st, 422, "{body}: {b}");
        assert!(fields(&b).contains(&field.to_string()), "{b}");
    }
    let (st, b) = e.read.post(&path, post("c", "x")).await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (403, Some("insufficient_scope"))
    );
    let (st, _) = e.chat.post("/sessions/9999/messages", post("c", "x")).await;
    assert_eq!(st, 404);
    let (st, _) = e.read.get("/sessions/9999/messages").await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn message_body_limit() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let path = format!("/sessions/{s}/messages");
    let (st, b) = e
        .chat
        .post(&path, post("big", &"x".repeat(1024 * 1024 + 1)))
        .await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (413, Some("payload_too_large"))
    );
    let (st, _) = e
        .chat
        .post(&path, post("edge", &"x".repeat(1024 * 1024)))
        .await;
    assert_eq!(st, 201, "exactly 1 MiB is accepted");
}

#[tokio::test(flavor = "multi_thread")]
async fn history_paginate_before_limit() {
    let e = Env::start().await;
    let s = e.session("x").await;
    for i in 0..7 {
        e.chat
            .make(
                &format!("/sessions/{s}/messages"),
                post(&format!("c{i}"), &format!("m{i}")),
            )
            .await;
    }
    let mut seen: Vec<i64> = Vec::new();
    let mut before: Option<i64> = None;
    for _ in 0..40 {
        let q = before.map_or(String::new(), |b| format!("&before={b}"));
        let (st, page) = e
            .read
            .get(&format!("/sessions/{s}/messages?limit=3{q}"))
            .await;
        assert_eq!(st, 200);
        let page = page.as_array().unwrap();
        assert!(page.len() <= 3);
        let Some(last) = page.last() else { break };
        let ids: Vec<i64> = page.iter().map(|m| m["id"].as_i64().unwrap()).collect();
        assert!(
            ids.windows(2).all(|w| w[0] > w[1]),
            "newest to oldest: {ids:?}"
        );
        seen.extend(&ids);
        before = last["id"].as_i64();
    }
    let mut sorted = seen.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(sorted, seen, "no duplicates and no order gaps");
    assert_eq!(user_ids(&e, s).await.len(), 7);
    for bad in ["limit=0", "limit=201", "limit=abc", "before=x", "before=0"] {
        let (st, _) = e.read.get(&format!("/sessions/{s}/messages?{bad}")).await;
        assert_eq!(st, 400, "{bad}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_clear_removes_messages() {
    let e = Env::start().await;
    let (a, b) = (e.session("a").await, e.session("b").await);
    for (s, c) in [(a, "1"), (a, "2"), (b, "1")] {
        e.chat
            .make(&format!("/sessions/{s}/messages"), post(c, "x"))
            .await;
    }
    let (st, _) = e
        .read
        .post(&format!("/sessions/{a}/clear"), json!({}))
        .await;
    assert_eq!(st, 403);
    let (st, _) = e
        .chat
        .post(&format!("/sessions/{a}/clear"), json!({}))
        .await;
    assert_eq!(st, 204);
    assert!(user_ids(&e, a).await.is_empty());
    assert_eq!(
        user_ids(&e, b).await.len(),
        1,
        "the other session is intact"
    );
    let (st, s) = e.read.get(&format!("/sessions/{a}")).await;
    assert_eq!((st, s["id"].as_i64()), (200, Some(a)));
    let (st, _) = e.chat.post("/sessions/9999/clear", json!({})).await;
    assert_eq!(st, 404);
}
