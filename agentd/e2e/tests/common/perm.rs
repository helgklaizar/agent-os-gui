//! Helpers for the permission end-to-end tests: session, agent permission request, decision, log.
#![allow(dead_code)]

use std::time::Duration;

use core_client::api::Frame;
use serde_json::{Value, json};

use super::{Api, Env, Reply};

pub async fn session(e: &Env, persona: Option<i64>) -> i64 {
    let body = persona.map_or(
        json!({"name": "s"}),
        |p| json!({"name": "s", "persona_id": p}),
    );
    e.chat.make("/sessions", body).await["id"].as_i64().unwrap()
}

/// The agent asks to delete session `s`: an open permission request (`202`).
pub async fn ask_delete(e: &Env, s: i64) -> Value {
    let (st, req) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 202, "{req}");
    req
}

pub async fn decide(api: &Api, id: i64, body: Value) -> Reply {
    api.post(&format!("/permission-requests/{id}/decision"), body)
        .await
}

pub async fn db_decision(e: &mut Env, id: i64) -> (Option<String>, Option<String>) {
    e.inspect(move |c| {
        c.query_row(
            "SELECT decision, result FROM actions_log WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    })
    .await
}

/// Outcome of a dispatcher turn: agent texts and the tool message.
pub async fn finished(e: &Env, s: i64) -> (Vec<String>, Option<Value>) {
    let m = e.wait_agent_done(s, 2).await;
    let agent = m
        .iter()
        .filter(|x| x["role"] == "agent")
        .map(|x| x["body_md"].as_str().unwrap().to_string())
        .collect();
    (agent, m.iter().find(|x| x["role"] == "tool").cloned())
}

pub type Row = (String, Option<String>, Option<String>);

/// Agent log entries for an action: `(action, decision, outcome)`.
pub async fn agent_log(e: &mut Env, action: &'static str) -> Vec<Row> {
    e.inspect(move |c| {
        c.prepare("SELECT action, decision, result FROM actions_log WHERE action = ?1 AND actor = 'agent' ORDER BY id")?
            .query_map([action], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    })
    .await
}

pub fn row(action: &str, decision: &str, result: &str) -> Row {
    (action.into(), Some(decision.into()), Some(result.into()))
}

pub fn mode_of(list: &Value, scope: &str, id: i64, action: &str) -> Option<String> {
    list.as_array()?
        .iter()
        .find(|p| p["scope"] == scope && p["scope_id"] == id && p["action"] == action)
        .map(|p| p["mode"].as_str().unwrap().to_string())
}

/// A `session.<id>` frame with a `kind` field.
pub fn is_kind(f: &Frame, kind: &str) -> bool {
    f.data["kind"] == kind
}

pub async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}
