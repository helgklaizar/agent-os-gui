//! A5-08: MCP client — call permissions, process environment, the `untrusted` label, timeout and restart, HTTP transport.
//! The server is `mcp-fake` (stdio and Streamable HTTP).

mod common;

use std::time::Duration;

use common::{
    Env, ask, body_of, finished, http_server, log_lines, log_of, set_mcp, setup, stdio_spec,
};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn mcp_tool_requires_permission() {
    let mut e = Env::start().await;
    setup(&e, json!({"echo": "send"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_echo").await;
    let (id, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "send", "{card}");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !log_lines(&e).iter().any(|l| l.starts_with("call")),
        "no call without a decision: {:?}",
        log_lines(&e)
    );
    assert_eq!(e.decide(id, json!({"decision": "allow"})).await.0, 200);
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
    assert_eq!(
        log_lines(&e).iter().filter(|l| *l == "call echo").count(),
        1
    );
    let rows = common::agent_log(&mut e, "send").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_denied_call_is_not_executed() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "send"}), json!({})).await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/send", json!({"mode": "deny"}))
            .await
            .0,
        200
    );
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_echo").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "permission_denied");
    assert!(!log_lines(&e).iter().any(|l| l.starts_with("call")));
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_unclassified_tool_defaults_to_ask() {
    let e = Env::start().await;
    setup(&e, json!({}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_ping").await;
    let (_, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "send", "{card}");
    assert!(!log_lines(&e).iter().any(|l| l.starts_with("call")));
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_read_class_runs_and_result_is_untrusted() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_echo").await;
    let (_, tool) = finished(&e, s).await;
    let tool = tool.unwrap();
    assert_eq!(tool["meta"]["code"], "ok", "{tool}");
    let body = body_of(&tool);
    for must in [
        "\"untrusted\":true",
        "\"origin\":\"from MCP server fake\"",
        "echo: hello",
    ] {
        assert!(body.contains(must), "{must} in {body}");
    }
    assert!(!body.contains('\n'));
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_injection_in_result_is_isolated() {
    let e = Env::start().await;
    setup(&e, json!({"evil": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_evil").await;
    let (_, tool) = finished(&e, s).await;
    let body = body_of(&tool.unwrap());
    assert!(body.contains("isolated by the observer"), "{body}");
    assert!(!body.contains("allow on shell"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_env_has_no_secrets() {
    let e = Env::start().await;
    setup(
        &e,
        json!({"env": "read"}),
        json!({"env": {"MCP_FAKE_LOG": log_of(&e).to_str().unwrap(), "FOO": "bar"}}),
    )
    .await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_env").await;
    let (_, tool) = finished(&e, s).await;
    let body = body_of(&tool.unwrap());
    assert!(
        body.contains("FOO,MCP_FAKE_LOG,PATH"),
        "only PATH and the declared variables: {body}"
    );
    for leaked in ["HOME", "AGENTD", "TOKEN", "KEY", "RUST_LOG"] {
        assert!(
            !body.contains(leaked),
            "{leaked} in the process environment: {body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_timeout_then_process_restarts() {
    let e = Env::start().await;
    setup(
        &e,
        json!({"slow": "read", "echo": "read"}),
        json!({"timeout_secs": 1}),
    )
    .await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_slow").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "timeout");
    // Background reconciliation may restart the server right after the timeout, before the next call: we count not "+1 to the snapshot",
    // but the order in the process log — start, the `slow` call, exactly one new start, then `echo`.
    let (_, s2) = e.model_session().await;
    ask(&e, s2, "action_mcp_echo").await;
    let (_, tool) = finished(&e, s2).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
    assert_eq!(
        log_lines(&e),
        ["start", "call slow", "start", "call echo"],
        "the process was restarted exactly once after the timeout"
    );
}

/// A short call timeout (`timeout_secs`) does not cut the handshake: a slow server start is not `unknown_tool`.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_slow_start_is_not_cut_by_call_timeout() {
    let e = Env::start().await;
    setup(
        &e,
        json!({"echo": "read"}),
        json!({"timeout_secs": 1, "env": {"MCP_FAKE_LOG": log_of(&e).to_str().unwrap(),
                                          "MCP_FAKE_START_DELAY_MS": "2500"}}),
    )
    .await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_echo").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
}

/// A server hung at startup (`timeout_secs` = 1) does not delay calls to another server: neither reconciliation, `resolve`,
/// nor the catalog wait for another server's handshake.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_hung_server_does_not_block_other_server() {
    let e = Env::start().await;
    let hung = stdio_spec(
        &e,
        json!({"echo": "read"}),
        json!({"name": "hung", "timeout_secs": 1,
               "env": {"MCP_FAKE_START_DELAY_MS": "20000"}}),
    );
    let quick = stdio_spec(&e, json!({"echo": "read"}), json!({"name": "quick"}));
    let (st, v) = set_mcp(&e, json!([hung, quick])).await;
    assert_eq!(st, 200, "{v}");
    let (_, s) = e.model_session().await;
    let t0 = std::time::Instant::now();
    ask(&e, s, "action_mcp_quick_echo").await;
    let (_, tool) = finished(&e, s).await;
    let took = t0.elapsed();
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
    assert!(
        took < Duration::from_secs(8),
        "a call to the fast server waited for the hung one: {took:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_unknown_tool_is_rejected() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_nope").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "unknown_tool");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_disabled_server_is_not_called() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), json!({"enabled": false})).await;
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_echo").await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "unknown_tool");
    assert!(log_lines(&e).is_empty(), "the process was not started");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_http_transport_call() {
    let e = Env::start().await;
    let (_srv, port) = http_server(&e);
    let spec = json!({"name": "remote", "transport": "http", "url": format!("http://127.0.0.1:{port}/mcp"),
                      "tools": {"echo": "read"}});
    let (st, v) = set_mcp(&e, json!([spec])).await;
    assert_eq!(st, 200, "{v}");
    let (_, s) = e.model_session().await;
    ask(&e, s, "action_mcp_http_echo").await;
    let (_, tool) = finished(&e, s).await;
    let body = body_of(&tool.unwrap());
    assert!(body.contains("echo: over the network"), "{body}");
}
