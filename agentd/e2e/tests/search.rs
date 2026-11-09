//! A6-08: `GET /search` — full-text search over sessions, messages and the `AGENT.md` and `memory/` files (FTS5 in `index.db`);
//! user query syntax is not executed; secrets and tool results never enter the index.

mod common;
mod openapi;

use std::fs;
use std::time::Duration;

use common::Env;
use openapi::{check, schemas};
use serde_json::{Value, json};

fn write_agentos(e: &Env, rel: &str, text: &str) {
    let p = e.home.join(".agentos").join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
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
    assert_eq!(st, 200, "{q}: {v}");
    v["items"].as_array().unwrap().clone()
}

async fn wait_agent(e: &Env, s: i64) {
    for _ in 0..200 {
        if e.messages(s).await.iter().any(|m| m["role"] == "agent") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the agent did not reply");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_finds_session_message_and_files() {
    let e = Env::start().await;
    let all = schemas();
    let (_, s) = e.model_session().await;
    e.chat
        .patch(
            &format!("/sessions/{s}"),
            json!({"name": "Shopping for bread"}),
        )
        .await;
    e.say(s, "c1", "buy rye bread tomorrow morning").await;
    wait_agent(&e, s).await;
    write_agentos(&e, "AGENT.md", "Favorite kind: Borodino bread");
    write_agentos(&e, "memory/pantry/note.md", "The pantry ran out of raisins");

    let hits = search(&e, "bread").await;
    hits.iter().for_each(|h| check("SearchHit", h, &all));
    let kinds: Vec<&str> = hits.iter().map(|h| h["kind"].as_str().unwrap()).collect();
    for want in ["session", "message", "file"] {
        assert!(kinds.contains(&want), "{want} in {hits:?}");
    }
    let msg = hits.iter().find(|h| h["kind"] == "message").unwrap();
    assert_eq!(msg["ref"]["session_id"], s, "{msg}");
    assert!(msg["snippet"].as_str().unwrap().contains("bread"), "{msg}");
    assert!(hits.iter().all(|h| h["source"] == "local"));
    let file = hits.iter().find(|h| h["kind"] == "file").unwrap();
    assert_eq!(file["ref"]["path"], "AGENT.md", "{file}");

    let nested = search(&e, "raisins").await;
    assert_eq!(
        nested[0]["ref"]["path"], "memory/pantry/note.md",
        "{nested:?}"
    );
    let prefix = search(&e, "cold").await;
    assert!(prefix.is_empty(), "no such word");
    let part = search(&e, "ry").await;
    assert!(!part.is_empty(), "search by word prefix");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_query_syntax_is_not_executed() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "word alpha and word beta").await;
    wait_agent(&e, s).await;
    let plain = search(&e, "alpha").await;
    assert!(!plain.is_empty());
    for q in [
        "\"",
        "alpha OR",
        "NEAR(",
        "*",
        "alpha*",
        "-alpha",
        "body:alpha",
        ")",
        "alpha\" OR \"beta",
        "alpha AND NOT beta",
        "^alpha",
        "alpha beta",
        "'; DROP TABLE docs; --",
    ] {
        let hits = search(&e, q).await;
        assert!(hits.len() <= 50, "{q}");
    }
    // Operators are plain words: "NOT" and "OR" do not filter results by column or negation.
    let both = search(&e, "alpha beta").await;
    assert!(!both.is_empty(), "space-separated words mean AND");
    let junk = search(&e, "alpha zzznosuchword").await;
    assert!(junk.is_empty(), "all words must match");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_validation_scope_and_limit() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    for i in 0..4 {
        e.say(s, &format!("c{i}"), &format!("repeat number {i} topic"))
            .await;
    }
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for path in [
        "/search",
        "/search?q=",
        "/search?q=%20%20",
        "/search?q=x&limit=0",
        "/search?q=x&limit=51",
        "/search?q=x&limit=abc",
    ] {
        let (st, v) = e.read.get(path).await;
        assert_eq!(st, 422, "{path}: {v}");
    }
    let long = "x".repeat(201);
    assert_eq!(e.read.get(&format!("/search?q={long}")).await.0, 422);
    let (st, v) = e.read.get("/search?q=topic&limit=2").await;
    assert_eq!(st, 200);
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        2,
        "limit caps the result: {v}"
    );
    let raw = common::raw(&e.sock, "foreign", "GET", "/search?q=x", None, b"").await;
    assert_eq!(raw.status, 401, "a foreign token is rejected");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_does_not_index_tool_results_secrets_or_foreign_files() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.admin
        .put("/permissions/global/0/read", json!({"mode": "allow"}))
        .await;
    e.say(s, "c1", "#scn:action_read read the file").await;
    for _ in 0..200 {
        if e.messages(s).await.iter().any(|m| m["role"] == "tool") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        e.messages(s).await.iter().any(|m| m["role"] == "tool"),
        "tool result appeared"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let tool_ids: Vec<i64> = e
        .messages(s)
        .await
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    let hits = search(&e, "tool_result").await;
    assert!(
        hits.iter()
            .all(|h| !tool_ids.contains(&h["ref"]["message_id"].as_i64().unwrap_or(-1))),
        "tool results are not indexed: {hits:?}"
    );
    write_agentos(&e, "keys.env", "ANTHROPIC_KEY=secretvalue123");
    write_agentos(&e, "other/readme.md", "secretvalue123 outside the roots");
    write_agentos(&e, "memory/ok.md", "ordinary note");
    assert!(
        search(&e, "secretvalue123").await.is_empty(),
        "only AGENT.md and memory/"
    );
    assert!(!search(&e, "note").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn search_forgets_deleted_sessions_and_files() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "uniqueword in chat").await;
    wait_agent(&e, s).await;
    write_agentos(&e, "memory/tmp.md", "tempnote");
    assert!(!search(&e, "uniqueword").await.is_empty());
    assert!(!search(&e, "tempnote").await.is_empty());
    assert_eq!(e.chat.delete(&format!("/sessions/{s}")).await.0, 204);
    fs::remove_file(e.home.join(".agentos/memory/tmp.md")).unwrap();
    assert!(
        search(&e, "uniqueword").await.is_empty(),
        "messages of the deleted session are removed from the index"
    );
    assert!(
        search(&e, "tempnote").await.is_empty(),
        "deleted file is removed from the index"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_index_lives_in_its_own_file_with_private_mode() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "index file check").await;
    wait_agent(&e, s).await;
    search(&e, "index").await;
    let idx = e.home.join(".agentos/index.db");
    let mode = fs::metadata(&idx)
        .expect("index.db created")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_ne!(idx, e.db, "a separate file, not sessions.db");
}
