//! A6-09: the `memory_mcp` memory source (option): `memory.source = local | mcp_memory | both`; only the text goes into the request,
//! results are not copied into the DB, unavailability and slow replies do not break search (degrades to local).

mod common;

use std::time::Duration;

use common::{Env, FAKE, set_mcp};
use serde_json::{Value, json};

fn memory_mcp_spec(e: &Env) -> Value {
    json!({"name": "memory_mcp", "transport": "stdio", "command": FAKE, "args": [],
           "env": {"MCP_FAKE_LOG": common::log_of(e).to_str().unwrap()},
           "tools": {"search": "read", "doc_search": "read"}, "timeout_secs": 5})
}

async fn set_source(e: &Env, source: &str) {
    let (_, page) = e.admin.get("/settings/memory").await;
    let (st, v) = e
        .admin
        .patch_if_match(
            "/settings/memory",
            json!({"source": source}),
            page["rev"].as_i64().unwrap_or(0),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

async fn search(e: &Env, q: &str) -> Vec<Value> {
    let enc: String = q
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let (st, v) = e.read.get(&format!("/search?q={enc}")).await;
    assert_eq!(st, 200, "{v}");
    v["items"].as_array().unwrap().clone()
}

fn calls(e: &Env) -> Vec<String> {
    common::log_lines(e)
        .into_iter()
        .filter(|l| l.starts_with("call search"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_local_never_calls_memory_mcp() {
    let e = Env::start().await;
    assert_eq!(set_mcp(&e, json!([memory_mcp_spec(&e)])).await.0, 200);
    let hits = search(&e, "bread").await;
    assert!(hits.iter().all(|h| h["source"] == "local"), "{hits:?}");
    assert!(
        calls(&e).is_empty(),
        "by default (local) memory_mcp is not queried"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_mcp_memory_sends_only_query_text() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "private user message about bread").await;
    assert_eq!(set_mcp(&e, json!([memory_mcp_spec(&e)])).await.0, 200);
    set_source(&e, "mcp_memory").await;
    let hits = search(&e, "bread").await;
    let remote: Vec<&Value> = hits
        .iter()
        .filter(|h| h["source"] == "mcp_memory")
        .collect();
    assert_eq!(remote.len(), 1, "{hits:?}");
    assert_eq!(
        (remote[0]["kind"].as_str(), remote[0]["title"].as_str()),
        (Some("memory"), Some("Fact from memory_mcp"))
    );
    assert!(
        hits.iter()
            .filter(|h| h["kind"] == "memory")
            .all(|h| h["source"] == "mcp_memory"),
        "memory records come from memory_mcp, other kinds are local: {hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h["kind"] == "message" && h["source"] == "local")
    );
    let sent = calls(&e);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let args: Value = serde_json::from_str(sent[0].trim_start_matches("call search ")).unwrap();
    assert_eq!(
        args,
        json!({"limit": 20, "query": "bread"}),
        "only the query text went to memory_mcp"
    );
    assert!(!sent[0].contains("private"), "user data does not leave");
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_both_merges_and_marks_sources() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "note about bread").await;
    for _ in 0..100 {
        if e.messages(s).await.iter().any(|m| m["role"] == "agent") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(set_mcp(&e, json!([memory_mcp_spec(&e)])).await.0, 200);
    set_source(&e, "both").await;
    let hits = search(&e, "bread").await;
    let sources: std::collections::BTreeSet<&str> =
        hits.iter().map(|h| h["source"].as_str().unwrap()).collect();
    assert_eq!(
        sources,
        ["local", "mcp_memory"].into_iter().collect(),
        "{hits:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_degrades_when_memory_mcp_is_unavailable() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "uniqueword in chat").await;
    for _ in 0..100 {
        if e.messages(s).await.iter().any(|m| m["role"] == "agent") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The server does not start at all: search answers with local results.
    let broken = json!({"name": "memory_mcp", "transport": "stdio", "command": "/nonexistent/mcp", "tools": {}});
    assert_eq!(set_mcp(&e, json!([broken])).await.0, 200);
    set_source(&e, "both").await;
    let hits = search(&e, "uniqueword").await;
    assert!(hits.iter().any(|h| h["source"] == "local"), "{hits:?}");
    // No data appeared in sessions.db: memory_mcp results are not copied.
    let n: i64 = e
        .inspect(|c| c.query_row("SELECT count(*) FROM memory_facts", [], |r| r.get(0)))
        .await;
    assert_eq!(n, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_stops_calling_after_three_failures() {
    let e = Env::start().await;
    assert_eq!(set_mcp(&e, json!([memory_mcp_spec(&e)])).await.0, 200);
    set_source(&e, "mcp_memory").await;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let hits = search(&e, "slowly").await;
        assert!(hits.is_empty(), "{hits:?}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "reply not longer than the threshold: {:?}",
            started.elapsed()
        );
    }
    assert_eq!(
        calls(&e).len(),
        3,
        "after three failures in a row memory_mcp is not queried for a while"
    );
}
