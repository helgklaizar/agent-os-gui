//! A5-06: connecting channel accounts (`ConnectFlow`), the sample channel, outgoing messages through permissions, unknown outcome.

mod common;

use std::time::Duration;

use common::Env;
use core_client::api::FrameKind;
use serde_json::{Value, json};

const KIND: &str = "scripted-channel";

async fn chan(e: &Env, body: Value) -> Value {
    let (st, v) = e.admin.post("/_test/channel", body).await;
    assert_eq!(st, 200, "{v}");
    v
}

async fn account(e: &Env) -> i64 {
    e.admin
        .make("/accounts", json!({"type": KIND, "label": "channel"}))
        .await["id"]
        .as_i64()
        .unwrap()
}

async fn status_of(e: &Env, id: i64) -> String {
    let (_, list) = e.read.get("/accounts").await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_flow_qr_status() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "qr"})).await;
    let a = account(&e).await;
    let mut ws = e.ws();
    ws.subscribe("connections");
    let (st, flow) = e
        .admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    assert_eq!(st, 202, "{flow}");
    assert_eq!(flow["account_id"], a);
    assert_eq!(
        (flow["kind"].as_str(), flow["status"].as_str()),
        (Some("qr"), Some("waiting_user"))
    );
    assert!(
        flow["qr_data"].as_str().is_some_and(|s| !s.is_empty()),
        "{flow}"
    );
    assert!(flow["expires_at"].as_i64().unwrap() > 0);
    assert_eq!(status_of(&e, a).await, "qr", "number unlinked, QR needed");
    let frames = ws.until(Duration::from_secs(10), |f| {
        f.topic == "connections" && f.data["status"] == "qr"
    });
    assert_eq!(frames.last().unwrap().data["account_id"], a);
    chan(&e, json!({"scanned": true, "refresh": a})).await;
    assert_eq!(status_of(&e, a).await, "connected");
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "connections" && f.data["status"] == "connected"
    });
    assert_eq!(f.last().unwrap().kind, FrameKind::Delta);
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_flow_token_waits_for_secret() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "token"})).await;
    let a = account(&e).await;
    let (st, flow) = e
        .admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    assert_eq!(st, 202, "{flow}");
    assert_eq!(
        (flow["kind"].as_str(), flow["status"].as_str()),
        (Some("token"), Some("waiting_user"))
    );
    assert_eq!(status_of(&e, a).await, "disconnected");
    let (st, _) = e
        .admin
        .put(
            &format!("/secrets/account.{a}"),
            json!({"value": "123456:test-token"}),
        )
        .await;
    assert_eq!(st, 200);
    for _ in 0..100 {
        if status_of(&e, a).await == "connected" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the account did not connect after the token");
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_flow_failed_token_is_error() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "token", "verify": "rejected"})).await;
    let a = account(&e).await;
    e.admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    e.admin
        .put(
            &format!("/secrets/account.{a}"),
            json!({"value": "bad-token"}),
        )
        .await;
    for _ in 0..100 {
        if status_of(&e, a).await == "error" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a wrong token must give error");
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_delete_disconnects() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "qr"})).await;
    let a = account(&e).await;
    e.admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    chan(&e, json!({"scanned": true, "refresh": a})).await;
    assert_eq!(status_of(&e, a).await, "connected");
    assert_eq!(
        e.admin.delete(&format!("/accounts/{a}/connect")).await.0,
        204
    );
    assert_eq!(status_of(&e, a).await, "disconnected");
    assert_eq!(
        e.admin.delete(&format!("/accounts/{a}/connect")).await.0,
        204,
        "repeat is harmless"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_errors_and_scope() {
    let e = Env::start().await;
    assert_eq!(
        e.admin.post("/accounts/9999/connect", json!({})).await.0,
        404
    );
    assert_eq!(e.admin.delete("/accounts/9999/connect").await.0, 404);
    let model = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "m"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let (st, v) = e
        .admin
        .post(&format!("/accounts/{model}/connect"), json!({}))
        .await;
    assert_eq!(st, 409, "the model has no channel: {v}");
    chan(&e, json!({"mode": "qr"})).await;
    let a = account(&e).await;
    assert_eq!(
        e.chat
            .post(&format!("/accounts/{a}/connect"), json!({}))
            .await
            .0,
        403
    );
    assert_eq!(
        e.read.delete(&format!("/accounts/{a}/connect")).await.0,
        403
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_health_indicator() {
    let e = Env::start().await;
    chan(&e, json!({"mode": "qr"})).await;
    let a = account(&e).await;
    let ind = format!("channel.{a}");
    let mut ws = e.ws();
    ws.subscribe("bar.indicators");
    e.admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "bar.indicators" && f.data["id"] == ind
    });
    assert_eq!(
        f.last().unwrap().data["state"],
        "bad",
        "QR = number unlinked = red"
    );
    chan(&e, json!({"scanned": true, "refresh": a})).await;
    let f = ws.until(Duration::from_secs(10), |f| {
        f.topic == "bar.indicators" && f.data["id"] == ind && f.data["state"] == "ok"
    });
    assert!(!f.is_empty());
}
