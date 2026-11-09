//! Accounts through the API (`GET/POST/PATCH/DELETE /accounts`, `connect`), write permissions.

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn accounts_create_list_patch_delete_connect() {
    let r = rig().await;
    let id = new_account(&r, "wa", "Working").await;
    let list = r.get("/v1/accounts").await;
    let me = list
        .as_array()
        .and_then(|l| l.iter().find(|a| a["id"] == id))
        .cloned();
    let me = me.unwrap_or(Value::Null);
    assert_eq!(
        (
            me["status"].as_str(),
            me["label"].as_str(),
            me["type"].as_str()
        ),
        (Some("disconnected"), Some("Working"), Some("wa"))
    );
    assert!(me.get("secret_ref").is_none(), "secret_ref is not returned");
    assert!(me["created_at"].is_number());
    let p = r
        .call(
            "PATCH",
            &format!("/v1/accounts/{id}"),
            Some(json!({"label": "New"})),
        )
        .await;
    assert_eq!(
        (p.status, p.json()["label"].as_str().map(str::to_string)),
        (200, Some("New".into()))
    );
    let bad = r
        .call(
            "POST",
            "/v1/accounts",
            Some(json!({"type": "", "label": "x"})),
        )
        .await;
    assert_eq!(bad.status, 422);
    let bad = r
        .call("POST", "/v1/accounts", Some(json!({"type": "wa"})))
        .await;
    assert_eq!(bad.status, 422);
    assert_eq!(
        r.call("PATCH", "/v1/accounts/9999", Some(json!({"label": "x"})))
            .await
            .status,
        404
    );
    // connecting: wa -> qr, gmail -> oauth, anthropic -> token
    let flow = r
        .call("POST", &format!("/v1/accounts/{id}/connect"), None)
        .await;
    assert_eq!(flow.status, 202, "{}", text(&flow));
    let f = flow.json();
    assert_eq!(
        (
            f["kind"].as_str(),
            f["status"].as_str(),
            f["account_id"].as_i64()
        ),
        (Some("qr"), Some("pending"), Some(id))
    );
    assert!(f["qr_data"].is_string());
    let after = r.get("/v1/accounts").await;
    let st = after
        .as_array()
        .and_then(|l| l.iter().find(|a| a["id"] == id))
        .map(|a| a["status"].clone());
    assert_eq!(st, Some(json!("qr")));
    assert_eq!(
        r.call("DELETE", &format!("/v1/accounts/{id}/connect"), None)
            .await
            .status,
        204
    );
    let gm = new_account(&r, "gmail", "g").await;
    assert_eq!(
        r.call("POST", &format!("/v1/accounts/{gm}/connect"), None)
            .await
            .json()["kind"],
        "oauth"
    );
    let an = new_account(&r, "anthropic", "a").await;
    assert_eq!(
        r.call("POST", &format!("/v1/accounts/{an}/connect"), None)
            .await
            .json()["kind"],
        "token"
    );
    assert_eq!(
        r.call("POST", "/v1/accounts/9999/connect", None)
            .await
            .status,
        404
    );
    // deletion (soft): the account disappears from the list
    assert_eq!(
        r.call("DELETE", &format!("/v1/accounts/{an}"), None)
            .await
            .status,
        204
    );
    let list = r.get("/v1/accounts").await;
    assert!(
        list.as_array()
            .is_some_and(|l| l.iter().all(|a| a["id"] != an))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_admin_only_for_writes() {
    let r = rig().await;
    let chat = r.token(Scope::Chat, Kind::External);
    let read = r.get("/v1/accounts").await;
    assert!(read.is_array());
    for (m, p, b) in [
        (
            "POST",
            "/v1/accounts",
            Some(json!({"type": "wa", "label": "x"})),
        ),
        ("PATCH", "/v1/accounts/1", Some(json!({"label": "x"}))),
        ("POST", "/v1/accounts/1/connect", None),
        ("DELETE", "/v1/accounts/1/connect", None),
    ] {
        let w = r.call_as(&chat, m, p, b).await;
        assert_eq!(
            (w.status, code(&w)),
            (403, "insufficient_scope".into()),
            "{m} {p}"
        );
    }
}
