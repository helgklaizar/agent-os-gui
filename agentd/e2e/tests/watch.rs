//! A7-03: the model-based observer (`agent-watch`, model `claude-haiku-5-5` via the core provider; in the test the
//! `scripted` provider, model `scripted:watch`). Rules are the first line and fire when the model is available; the model supplements them;
//! when the model is unavailable, dangerous actions are closed (fail-closed).

mod common;

use std::time::Duration;

use common::{Env, finished};
use serde_json::json;

/// A session with a scripted model and `shell` allowed (the permission ensures only the observer creates the card).
async fn session(e: &Env) -> i64 {
    let (_, s) = e.model_session().await;
    let project = e.home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let (st, b) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"path": project}))
        .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    s
}

async fn health_status(e: &Env) -> String {
    let (_, h) = e.read.get("/health").await;
    h["status"].as_str().unwrap_or("").to_string()
}

/// Waits until the heartbeat confirms the model is available (`/health` = `ok`).
async fn wait_available(e: &Env) {
    for _ in 0..100 {
        if health_status(e).await == "ok" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the observer did not become available");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_model_flags_what_rules_miss() {
    let e = Env::start_watch(1).await;
    let s = session(&e).await;
    wait_available(&e).await;
    e.say(s, "c1", "#scn:action_shellrun echo MODEL_DANGEROUS")
        .await;
    let (id, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["source"], "observer", "{card}");
    assert_eq!(card["meta"]["request"]["kind"], "dangerous_cmd", "{card}");
    assert_eq!(e.decide(id, json!({"decision": "deny"})).await.0, 200);
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "denied");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_safe_verdict_does_not_ask() {
    let e = Env::start_watch(1).await;
    let s = session(&e).await;
    wait_available(&e).await;
    e.say(s, "c1", "#scn:action_shellrun echo hello").await;
    let (_, tool) = finished(&e, s).await;
    // Without a runner the command is not executed, but there is no card: the gate let it through.
    assert_eq!(tool.unwrap()["meta"]["code"], "executor_unavailable");
    assert!(e.messages(s).await.iter().all(|m| m["role"] != "system"));
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_rules_first_line_still_apply() {
    let e = Env::start_watch(1).await;
    let s = session(&e).await;
    wait_available(&e).await;
    // The model would say "safe", but the rule blocks it.
    e.say(s, "c1", "#scn:action_shellrun rm -rf / # MODEL_SAFE")
        .await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "blocked_by_observer");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_unavailable_fail_closed() {
    let e = Env::start_watch(3).await;
    let s = session(&e).await;
    wait_available(&e).await;
    // The model fails on this request: the action is closed with a card; the observer is unavailable until the next heartbeat.
    e.say(s, "c1", "#scn:action_shellrun echo MODEL_DOWN").await;
    let (id, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["source"], "observer", "{card}");
    assert_eq!(health_status(&e).await, "degraded");
    assert_eq!(e.decide(id, json!({"decision": "deny"})).await.0, 200);
    // The next heartbeat succeeds — the observer returns.
    wait_available(&e).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_without_model_account_is_unavailable() {
    let e = Env::start_watch(1).await;
    e.chat.make("/sessions", json!({"name": "chat"})).await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        health_status(&e).await,
        "degraded",
        "there is no model account"
    );
}
