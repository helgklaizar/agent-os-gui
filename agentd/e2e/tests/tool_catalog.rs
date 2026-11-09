//! A-core-100 (A4-13 item 4, A5 "MCP tool catalog"): the model sees the catalog — built-in actions and the tools
//! of connected MCP servers (names `mcp__<server>__<tool>`). The `tools` scenario replies with the names from the request.

mod common;

use common::{Env, setup};
use serde_json::json;

async fn names_seen_by_model(e: &Env, s: i64) -> String {
    e.say(s, "c1", "#scn:tools what can you do").await;
    let m = e.wait_agent_done(s, 1).await;
    m.iter()
        .filter(|x| x["role"] == "agent")
        .map(|x| x["body_md"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread")]
async fn model_sees_mcp_tools_and_builtins() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    let text = names_seen_by_model(&e, s).await;
    let list = text
        .split_once("TOOLS[")
        .and_then(|(_, r)| r.split_once(']'))
        .map_or("", |(l, _)| l);
    for name in ["mcp__fake__echo", "mcp__fake__ping", "shell", "browse"] {
        assert!(
            list.split(',').any(|n| n == name),
            "{name} in the catalog: {text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_mcp_server_is_not_in_catalog() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let text = names_seen_by_model(&e, s).await;
    assert!(!text.contains("mcp__"), "{text}");
    assert!(
        text.contains("shell"),
        "built-ins are present even without MCP: {text}"
    );
}
