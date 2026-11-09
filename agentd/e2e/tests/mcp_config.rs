//! A5-08: MCP server setup on the `connections` page: field validation and server status in the `connections` topic.

mod common;

use std::time::Duration;

use common::{Env, FAKE, fields, set_mcp, setup, stdio_spec};
use core_client::api::FrameKind;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn mcp_settings_validation() {
    let e = Env::start().await;
    let ok = stdio_spec(&e, json!({"echo": "read"}), json!({}));
    assert_eq!(set_mcp(&e, json!([ok.clone()])).await.0, 200);
    for (bad, field) in [
        (
            json!({"name": "A b", "transport": "stdio", "command": FAKE}),
            "mcp[0].name",
        ),
        (
            json!({"name": "a__b", "transport": "stdio", "command": FAKE}),
            "mcp[0].name",
        ),
        (json!({"name": "x", "transport": "ftp"}), "mcp[0].transport"),
        (json!({"name": "x", "transport": "stdio"}), "mcp[0].command"),
        (
            json!({"name": "x", "transport": "http", "url": "file:///etc"}),
            "mcp[0].url",
        ),
        (
            json!({"name": "x", "transport": "stdio", "command": FAKE, "tools": {"echo": "launch"}}),
            "mcp[0].tools",
        ),
        (
            json!({"name": "x", "transport": "stdio", "command": FAKE, "env": {"AGENTD_TOKEN": "t"}}),
            "mcp[0].env",
        ),
        (
            json!({"name": "x", "transport": "stdio", "command": FAKE, "timeout_secs": 0}),
            "mcp[0].timeout_secs",
        ),
    ] {
        let (st, v) = set_mcp(&e, json!([bad])).await;
        assert_eq!(st, 422, "{field}: {v}");
        assert!(fields(&v).contains(&field.to_string()), "{field}: {v}");
    }
    let (st, v) = set_mcp(&e, json!([ok.clone(), ok])).await;
    assert_eq!(st, 422, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_status_in_connections_topic() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("connections");
    let bad = json!({"name": "broken", "transport": "stdio", "command": "/nonexistent/mcp"});
    let good = stdio_spec(&e, json!({"echo": "read"}), json!({}));
    let (st, v) = set_mcp(&e, json!([good, bad])).await;
    assert_eq!(st, 200, "{v}");
    let frames = ws.until(Duration::from_secs(15), |f| {
        f.data["kind"] == "mcp" && f.data["server"] == "broken"
    });
    let last = frames.last().unwrap();
    assert_eq!(last.kind, FrameKind::Delta);
    assert_eq!(last.data["status"], "down", "{:?}", last.data);
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_status_connected_reports_tool_count() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("connections");
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let frames = ws.until(Duration::from_secs(15), |f| {
        f.data["kind"] == "mcp" && f.data["server"] == "fake" && f.data["status"] == "connected"
    });
    assert_eq!(frames.last().unwrap().data["tools"], 8);
}
