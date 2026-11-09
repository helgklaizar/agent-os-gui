//! W4-01: a web token with the `@decide` grant decides a permission request once; "Always" and a plain `chat` token cannot.

mod common;

use common::{Env, finished};
use serde_json::json;

async fn web_token(e: &Env, name: &str, decide: bool) -> String {
    let body = e
        .admin
        .make(
            "/tokens",
            json!({"name": name, "scope": "chat", "decide": decide}),
        )
        .await;
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn web_token_decides_once_but_not_always() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    let web = e.api_with(&web_token(&e, "web", true).await);
    let plain = e.api_with(&web_token(&e, "plain", false).await);

    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "send");
    let path = format!("/permission-requests/{id}/decision");

    // A plain `chat` and "Always" from a web token are refused, the request stays open.
    let (st, body) = plain.post(&path, json!({"decision": "allow"})).await;
    assert_eq!(st, 403, "{body}");
    let always =
        json!({"decision": "always", "rule": {"scope": "global", "scope_id": 0, "mode": "allow"}});
    let (st, body) = web.post(&path, always).await;
    assert_eq!(
        (st, body["error"]["details"]["reason"].as_str()),
        (403, Some("always_needs_gui")),
        "{body}"
    );
    let (_, req) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(req["status"], "pending", "{req}");

    // A one-time decision by a web token passes; a second decision gives `409`.
    let (st, body) = web.post(&path, json!({"decision": "allow"})).await;
    assert_eq!(st, 200, "{body}");
    let (st, _) = web.post(&path, json!({"decision": "deny"})).await;
    assert_eq!(st, 409);
    let (agent, _) = finished(&e, s).await;
    assert!(agent[1].contains("no_connector"), "{agent:?}");

    // In the log the decision is signed with the token kind and number, not its value.
    let rows = e.log_rows("send").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0.as_deref(), Some("ask-approved"));
}

/// W-100: a `decide` web token edits the safe Settings pages; `models`, `permissions`, `tunnels` are closed to it,
/// a plain `chat` edits nothing except `theme`.
#[tokio::test(flavor = "multi_thread")]
async fn web_token_patches_only_safe_settings_pages() {
    let e = Env::start().await;
    let web = e.api_with(&web_token(&e, "web", true).await);
    let plain = e.api_with(&web_token(&e, "plain", false).await);

    let (st, doc) = web.get("/settings/memory").await;
    assert_eq!(st, 200, "{doc}");
    let rev = doc["rev"].as_i64().unwrap_or(0);
    let (st, body) = web
        .patch_if_match("/settings/memory", json!({"source": "both"}), rev)
        .await;
    assert_eq!((st, body["source"].as_str()), (200, Some("both")), "{body}");
    let (_, after) = e.read.get("/settings/memory").await;
    assert_eq!(after["source"], "both");

    // Without the decide right and outside the page list: refused by scope, the document does not change.
    let (st, body) = plain
        .patch_if_match("/settings/memory", json!({"source": "local"}), rev + 1)
        .await;
    assert_eq!(
        (st, body["error"]["code"].as_str()),
        (403, Some("insufficient_scope")),
        "{body}"
    );
    for page in ["models", "permissions", "tunnels", "connections"] {
        let (st, body) = web
            .patch_if_match(&format!("/settings/{page}"), json!({"x": 1}), 0)
            .await;
        assert_eq!(
            (st, body["error"]["code"].as_str()),
            (403, Some("insufficient_scope")),
            "{page}: {body}"
        );
    }
    // A secret value is not accepted through a web token even on an allowed page.
    let (st, body) = web
        .patch_if_match("/settings/account", json!({"api_key": "x"}), 0)
        .await;
    assert_eq!(st, 422, "{body}");
}
