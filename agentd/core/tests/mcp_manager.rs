//! Panic while connecting an MCP server does not leave it in `connecting`;
//! `catalog` waits for an in-progress connection no longer than its server's `timeout_secs`. Time is controlled (`start_paused`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentd_core::{
    Mcp, McpCallResult, McpConnector, McpError, McpServerSpec, McpSession, Notifier, PortFuture,
    RemoteTool, SettingsDoc, SettingsError, SettingsStore,
};
use serde_json::{Value, json};
use tokio::time::Instant;

struct Page(Value);

impl SettingsStore for Page {
    fn get<'a>(&'a self, _: &'a str) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        let doc = self.0.clone();
        Box::pin(async move { Ok(SettingsDoc { doc, rev: 1 }) })
    }
    fn patch<'a>(
        &'a self,
        _: &'a str,
        _: Value,
        _: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async { Err(SettingsError::ReadOnly) })
    }
    fn get_list<'a>(
        &'a self,
        _: &'a str,
        _: Value,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async { Err(SettingsError::Unavailable) })
    }
    fn put_list<'a>(
        &'a self,
        _: &'a str,
        _: Value,
        _: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async { Err(SettingsError::Unavailable) })
    }
    fn revisions(&self) -> PortFuture<'_, Result<BTreeMap<String, i64>, SettingsError>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
}

struct Quiet;

impl Notifier for Quiet {
    fn publish(&self, _: &str, _: Value) {}
}

struct OneTool;

impl McpSession for OneTool {
    fn list_tools(&self) -> PortFuture<'_, Result<Vec<RemoteTool>, McpError>> {
        Box::pin(async {
            Ok(vec![RemoteTool {
                name: "ping".to_string(),
                description: String::new(),
                schema: json!({"type": "object"}),
            }])
        })
    }
    fn call<'a>(
        &'a self,
        _: &'a str,
        _: &'a Value,
    ) -> PortFuture<'a, Result<McpCallResult, McpError>> {
        Box::pin(async {
            Ok(McpCallResult {
                text: String::new(),
                is_error: false,
            })
        })
    }
}

/// The first "flaky" connection panics, later ones succeed; "hung" never answers; the others succeed at once.
struct Connector {
    flaky_calls: AtomicUsize,
}

impl McpConnector for Connector {
    fn connect<'a>(
        &'a self,
        spec: &'a McpServerSpec,
    ) -> PortFuture<'a, Result<Arc<dyn McpSession>, McpError>> {
        Box::pin(async move {
            match spec.name.as_str() {
                "hung" => std::future::pending().await,
                "flaky" if self.flaky_calls.fetch_add(1, Ordering::SeqCst) == 0 => {
                    panic!("connector panic (test)")
                }
                _ => Ok(Arc::new(OneTool) as Arc<dyn McpSession>),
            }
        })
    }
}

fn mcp(servers: Value) -> Mcp {
    Mcp::new(
        Arc::new(Page(json!({"mcp": servers}))),
        Arc::new(Connector {
            flaky_calls: AtomicUsize::new(0),
        }),
        Arc::new(Quiet),
    )
}

fn stdio(name: &str, timeout: u64) -> Value {
    json!({"name": name, "transport": "stdio", "command": "x", "timeout_secs": timeout})
}

#[tokio::test(start_paused = true)]
async fn panic_in_connect_does_not_leave_server_connecting() {
    let m = mcp(json!([stdio("flaky", 5)]));
    m.sync().await;
    assert!(m.catalog().await.is_empty(), "no server after the panic");
    // The retry time (RETRY_AFTER = 5 s) has passed: reconciliation must connect the server again.
    tokio::time::advance(Duration::from_secs(6)).await;
    m.sync().await;
    let names: Vec<String> = m.catalog().await.into_iter().map(|t| t.name).collect();
    assert_eq!(names, vec!["mcp__flaky__ping".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn catalog_waits_for_hung_connect_no_longer_than_timeout_secs() {
    let m = mcp(json!([stdio("hung", 1), stdio("ok", 1)]));
    let t0 = Instant::now();
    let names: Vec<String> = m.catalog().await.into_iter().map(|t| t.name).collect();
    let waited = t0.elapsed();
    assert_eq!(names, vec!["mcp__ok__ping".to_string()]);
    assert!(waited >= Duration::from_secs(1), "waited {waited:?}");
    assert!(waited < Duration::from_secs(5), "waited {waited:?}");
}
