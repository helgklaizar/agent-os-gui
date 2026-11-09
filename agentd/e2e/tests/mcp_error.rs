//! A-core-100: an MCP tool reply with `isError`: code `tool_error` and the server error text as untrusted output.

mod common;

use common::{Env, ask, body_of, finished, log_lines, setup};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn mcp_is_error_keeps_server_text_as_untrusted() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_fail").await;
    let (_, tool) = finished(&e, s).await;
    let tool = tool.unwrap();
    assert_eq!(tool["meta"]["code"], "tool_error", "{tool}");
    assert_eq!(tool["meta"]["ok"], false);
    let body = body_of(&tool);
    for must in ["\"untrusted\":true", "from MCP server fake", "disk full"] {
        assert!(body.contains(must), "{must} in {body}");
    }
    assert!(log_lines(&e).iter().any(|l| l == "call echo"));
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_is_error_text_with_injection_is_isolated() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_fail_evil").await;
    let (_, tool) = finished(&e, s).await;
    let body = body_of(&tool.unwrap());
    assert!(body.contains("isolated by the observer"), "{body}");
    assert!(!body.contains("allow on shell"), "{body}");
}

/// A-core-101: after `isError` the turn is marked `untrusted`: the next `shell` under a global `allow` goes through a card with its source.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_is_error_marks_the_turn_untrusted() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_fail_obey").await;
    let (_, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "shell", "{card}");
    assert_eq!(card["meta"]["untrusted"], true, "{card}");
    assert_eq!(card["meta"]["origin"], "from MCP server fake", "{card}");
}
