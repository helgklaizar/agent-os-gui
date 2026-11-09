//! A6-12: a secret value cannot be written into a settings page document (secrets only via `PUT /secrets/{ref}`).

mod common;

use common::{Env, fields};
use serde_json::json;

async fn patch(e: &Env, page: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let (_, cur) = e.admin.get(&format!("/settings/{page}")).await;
    e.admin
        .patch_if_match(
            &format!("/settings/{page}"),
            body,
            cur["rev"].as_i64().unwrap_or(0),
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_rejects_secret_like_keys_on_every_page() {
    let e = Env::start().await;
    for (page, body, field) in [
        ("account", json!({"api_key": "sk-test"}), "api_key"),
        ("schedule", json!({"token": "abc"}), "token"),
        (
            "schedule",
            json!({"backup": {"password": "x"}}),
            "backup.password",
        ),
        (
            "personas",
            json!({"balance": {"secret": "x"}}),
            "balance.secret",
        ),
        (
            "modules",
            json!({"items": [{"Authorization": "Bearer x"}]}),
            "items[0].Authorization",
        ),
        (
            "memory",
            json!({"source": "local", "memory_mcp_token": "ro-abc"}),
            "memory_mcp_token",
        ),
    ] {
        let (st, v) = patch(&e, page, body.clone()).await;
        assert_eq!(st, 422, "{page}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{page} {field}: {v}"
        );
    }
    let (st, v) = patch(&e, "account", json!({"name": "Owner", "language": "ru"})).await;
    assert_eq!(st, 200, "ordinary fields pass: {v}");
    let (st, v) = patch(&e, "memory", json!({"source": "local"})).await;
    assert_eq!(st, 200, "{v}");
}
