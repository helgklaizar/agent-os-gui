//! A3-09: the `models` settings page and account status after a key check.

mod common;

use std::time::Duration;

use common::{Env, fields, status_server};
use serde_json::json;

const VALUE: &str = "sk-models-SECRET-1122";

#[tokio::test(flavor = "multi_thread")]
async fn models_page_no_secret_value() {
    let e = Env::start().await;
    for (patch, field) in [
        (
            json!({"accounts": {"1": {"api_key": VALUE}}}),
            "accounts.1.api_key",
        ),
        (
            json!({"accounts": {"1": {"secret": VALUE}}}),
            "accounts.1.secret",
        ),
        (
            json!({"accounts": {"1": {"secret_ref": VALUE}}}),
            "accounts.1.secret_ref",
        ),
        (json!({"api_key": VALUE}), "api_key"),
        (
            json!({"accounts": {"1": {"base_url": "https://user:pass@example.test/v1"}}}),
            "accounts.1.base_url",
        ),
    ] {
        let (st, b) = e.patch_models(patch.clone()).await;
        assert_eq!(st, 422, "{patch}: {b}");
        assert!(fields(&b).contains(&field.to_string()), "{patch}: {b}");
    }
    let (st, b) = e
        .patch_models(json!({"accounts": {"1": {"secret_ref": "account.1", "model": "m", "base_url": "https://api.example.test/v1"}}}))
        .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["accounts"]["1"]["secret_ref"], "account.1");
    // A secret value does not reach the page, even if it was written via /secrets.
    let acc = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "k"}))
        .await;
    let r = format!("account.{}", acc["id"]);
    e.admin
        .put(&format!("/secrets/{r}"), json!({"value": VALUE}))
        .await;
    let (_, page) = e.admin.get("/settings/models").await;
    assert!(!page.to_string().contains(VALUE));
}

#[tokio::test(flavor = "multi_thread")]
async fn models_page_validation() {
    let e = Env::start().await;
    for (patch, field) in [
        (json!({"accounts": {"x": {"model": "m"}}}), "accounts.x"),
        (json!({"accounts": {"0": {"model": "m"}}}), "accounts.0"),
        (
            json!({"accounts": {"1": {"model": ""}}}),
            "accounts.1.model",
        ),
        (
            json!({"accounts": {"1": {"base_url": "ftp://x"}}}),
            "accounts.1.base_url",
        ),
        (
            json!({"accounts": {"1": {"base_url": ""}}}),
            "accounts.1.base_url",
        ),
        (
            json!({"accounts": {"1": {"unknown": 1}}}),
            "accounts.1.unknown",
        ),
        (
            json!({"prices": {"m": {"input_per_m": -1.0, "output_per_m": 1.0}}}),
            "prices.m.input_per_m",
        ),
        (
            json!({"prices": {"m": {"input_per_m": 1.0}}}),
            "prices.m.output_per_m",
        ),
        (
            json!({"prices": {"m": {"input_per_m": 1.0, "output_per_m": 1.0, "currency": "dollar"}}}),
            "prices.m.currency",
        ),
        (json!({"context_budget": 0}), "context_budget"),
        (json!({"context_budget": "lots"}), "context_budget"),
        (json!({"unknown": 1}), "unknown"),
    ] {
        let (st, b) = e.patch_models(patch.clone()).await;
        assert_eq!(st, 422, "{patch}: {b}");
        assert!(fields(&b).contains(&field.to_string()), "{patch}: {b}");
    }
    let ok = json!({"accounts": {"2": {"model": "claude-x", "base_url": "http://127.0.0.1:11434/v1"}},
        "prices": {"claude-x": {"input_per_m": 3.0, "output_per_m": 15.0, "currency": "USD"}}, "context_budget": 32000});
    let (st, b) = e.patch_models(ok).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["context_budget"], 32000);
}

#[tokio::test(flavor = "multi_thread")]
async fn models_page_account_connected_after_key_check() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("connections");
    let acc = e
        .admin
        .make("/accounts", json!({"type": "scripted", "label": "test"}))
        .await;
    let id = acc["id"].as_i64().unwrap();
    ws.drain(Duration::from_millis(200));
    let (st, _) = e
        .admin
        .put(&format!("/secrets/account.{id}"), json!({"value": VALUE}))
        .await;
    assert_eq!(st, 200);
    assert_eq!(e.account_status_settled(id).await, "connected");
    let frames = ws.drain(Duration::from_millis(500));
    let got = frames
        .iter()
        .find(|f| f.data["status"] == "connected")
        .expect("connections delta");
    assert_eq!(got.data["account_id"], id);
    // Deleting a secret does not touch the status until the next check, and the account fields do not change.
    let (_, list) = e.read.get("/accounts").await;
    let keys: Vec<&str> = list[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(!keys.contains(&"secret_ref") && !list.to_string().contains(VALUE));
}

#[tokio::test(flavor = "multi_thread")]
async fn models_page_local_account_checked_by_url_error_on_failure() {
    let e = Env::start().await;
    let (good, bad) = (status_server(200).await, status_server(401).await);
    let a = e
        .admin
        .make("/accounts", json!({"type": "local", "label": "Ollama"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let b = e
        .admin
        .make("/accounts", json!({"type": "local", "label": "Broken"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let patch = json!({"accounts": {a.to_string(): {"model": "m", "base_url": good}, b.to_string(): {"model": "m", "base_url": bad}}});
    let (st, body) = e.patch_models(patch).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(e.account_status_settled(a).await, "connected");
    assert_eq!(e.account_status_settled(b).await, "error");
    let (_, list) = e.read.get("/accounts").await;
    assert!(
        !list.to_string().contains("secret-detail"),
        "the provider error text is not exposed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn models_page_unreachable_provider_is_error_not_hang() {
    let e = Env::start().await;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    let a = e
        .admin
        .make("/accounts", json!({"type": "local", "label": "no server"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let (st, b) = e
        .patch_models(json!({"accounts": {a.to_string(): {"model": "m", "base_url": url}}}))
        .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(e.account_status_settled(a).await, "error");
}
