//! `GET /search` on `fake-agentd` over a real socket.

mod common;
#[allow(dead_code)]
#[path = "contract/rawhttp.rs"]
mod rawhttp;

use common::*;
use fake_agentd::{Config, FakeAgentd};
use rawhttp::{Req, send};
use serde_json::{Value, json};

async fn start(seed: impl FnOnce(&fake_agentd::Db)) -> (tempfile::TempDir, FakeAgentd) {
    let (d, path) = temp_db_path();
    let db = open(&path);
    fixture::seed_search(&db);
    seed(&db);
    drop(db);
    let fa = FakeAgentd::start(Config::temp().no_ticker().db(path))
        .await
        .expect("fake-agentd");
    (d, fa)
}

async fn search(fa: &FakeAgentd, q: &str) -> Value {
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
    let r = send(
        fa.socket_path(),
        Req::new("GET", &format!("/v1/search?q={enc}&limit=20")).token(fa.admin_token()),
    )
    .await;
    assert_eq!(r.status, 200, "GET /search?q={q}");
    r.json()
}

fn hits(v: &Value) -> Vec<Value> {
    v["items"].as_array().cloned().unwrap_or_default()
}

fn has(v: &Value, kind: &str, title_part: &str) -> bool {
    hits(v).iter().any(|h| {
        h["kind"] == kind
            && (h["title"].as_str().unwrap_or("").contains(title_part)
                || h["snippet"].as_str().unwrap_or("").contains(title_part))
    })
}

/// The live switching path: `PATCH /v1/settings/memory` with `If-Match`.
async fn set_source(fa: &FakeAgentd, source: &str) {
    let got = send(
        fa.socket_path(),
        Req::new("GET", "/v1/settings/memory").token(fa.admin_token()),
    )
    .await;
    assert_eq!(got.status, 200, "GET settings/memory");
    let rev = got.json()["rev"].as_i64().unwrap_or(-1);
    let r = send(
        fa.socket_path(),
        Req::new("PATCH", "/v1/settings/memory")
            .token(fa.admin_token())
            .header("If-Match", &format!("\"{rev}\""))
            .json(json!({ "source": source })),
    )
    .await;
    assert_eq!(r.status, 200, "switching memory.source: {}", r.json());
}

#[tokio::test(flavor = "multi_thread")]
async fn search_finds_session_message_mail_memory_and_file() {
    let (_d, fa) = start(|_| {}).await;
    assert!(has(&search(&fa, "Startup").await, "session", "Startup"));
    assert!(has(&search(&fa, "migration").await, "message", "migration"));
    assert!(has(&search(&fa, "Hosting").await, "mail", "Hosting"));
    assert!(has(&search(&fa, "short").await, "memory", "short"));
    assert!(has(&search(&fa, "roadmap").await, "file", "roadmap"));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_hit_has_source_and_non_memory_is_local() {
    let (_d, fa) = start(|_| {}).await;
    let all = search(&fa, "e").await;
    assert!(!hits(&all).is_empty());
    for h in hits(&all) {
        let src = h["source"].as_str().unwrap_or("");
        assert!(
            ["local", "mcp_memory"].contains(&src),
            "source on every result: {h}"
        );
        if h["kind"] != "memory" {
            assert_eq!(src, "local", "{h}");
        }
        assert!(h["ref"].is_object(), "ref: {h}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_source_switch_changes_origin() {
    let (_d, fa) = start(|_| {}).await;
    let sources = |v: &Value| -> Vec<String> {
        hits(v)
            .iter()
            .filter(|h| h["kind"] == "memory")
            .map(|h| h["source"].as_str().unwrap_or("").to_string())
            .collect()
    };
    let local = sources(&search(&fa, "deploy").await);
    assert!(
        local.is_empty(),
        "local by default: there is no memory_mcp fact: {local:?}"
    );
    assert_eq!(sources(&search(&fa, "short").await), vec!["local"]);

    set_source(&fa, "mcp_memory").await;
    assert_eq!(sources(&search(&fa, "deploy").await), vec!["mcp_memory"]);
    assert!(
        sources(&search(&fa, "short").await).is_empty(),
        "local is not queried"
    );

    set_source(&fa, "both").await;
    assert_eq!(sources(&search(&fa, "deploy").await), vec!["mcp_memory"]);
    assert_eq!(sources(&search(&fa, "short").await), vec!["local"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_and_blank_queries_return_no_items() {
    let (_d, fa) = start(|_| {}).await;
    assert!(hits(&search(&fa, "").await).is_empty());
    assert!(hits(&search(&fa, "   ").await).is_empty());
    assert!(
        search(&fa, "").await["items"].is_array(),
        "items is an array"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hostile_queries_do_not_match_everything_or_break_the_db() {
    let (_d, fa) = start(|_| {}).await;
    assert!(
        hits(&search(&fa, "%").await).is_empty(),
        "% is not a LIKE pattern"
    );
    assert!(
        hits(&search(&fa, "_").await).is_empty(),
        "_ is not a LIKE pattern"
    );
    let inj = search(&fa, "'; DROP TABLE sessions; --").await;
    assert!(hits(&inj).is_empty());
    assert!(
        has(&search(&fa, "Startup").await, "session", "Startup"),
        "the table is alive"
    );
    let _ = search(&fa, &"y".repeat(5000)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hostile_content_is_returned_verbatim_as_json_string() {
    let evil = "<img src=x onerror=alert(1)> & \"quotes\"";
    let (_d, fa) = start(|db| {
        db.write(|w| {
            w.execute(
                "INSERT INTO messages(session_id, role, body_md, ts) VALUES (1, 'user', ?1, 99)",
                [evil],
            )
        })
        .expect("hostile message");
    })
    .await;
    let v = search(&fa, "onerror").await;
    let snip = hits(&v)
        .iter()
        .find(|h| h["kind"] == "message")
        .and_then(|h| h["snippet"].as_str().map(str::to_string))
        .unwrap_or_default();
    assert!(
        snip.contains(evil),
        "the core returns text as is, the view escapes: {snip}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn limit_is_respected_and_index_db_is_a_separate_file() {
    let (_d, fa) = start(|_| {}).await;
    let r = send(
        fa.socket_path(),
        Req::new("GET", "/v1/search?q=e&limit=2").token(fa.admin_token()),
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(hits(&r.json()).len() <= 2);
    let idx = fa.db_path().with_file_name("index.db");
    assert!(
        idx.exists(),
        "index.db is a separate file next to sessions.db"
    );
    assert_ne!(idx, fa.db_path());
}

#[tokio::test(flavor = "multi_thread")]
async fn search_requires_token() {
    let (_d, fa) = start(|_| {}).await;
    let r = send(fa.socket_path(), Req::new("GET", "/v1/search?q=a")).await;
    assert_eq!(r.status, 401);
}
