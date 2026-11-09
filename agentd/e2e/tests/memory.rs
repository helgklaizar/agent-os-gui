//! A6-07: memory: the `memory_write` tool through the dispatcher (permissions, journal), persona modes `own|shared|none`,
//! limits, reading facts into the reply context for the own persona only.

mod common;

use common::{Env, finished};
use serde_json::json;

async fn persona(e: &Env, name: &str, mode: &str) -> i64 {
    e.admin
        .make(
            "/personas",
            json!({"name": name, "color": "#abc", "mem_mode": mode}),
        )
        .await["id"]
        .as_i64()
        .unwrap()
}

/// Session with the `scripted` model and persona `p`.
async fn session_of(e: &Env, p: i64) -> i64 {
    let (_, s) = e.model_session().await;
    let (st, v) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"persona_id": p}))
        .await;
    assert_eq!(st, 200, "{v}");
    s
}

type Fact = (Option<i64>, Option<i64>, String, String);

async fn facts(e: &mut Env) -> Vec<Fact> {
    e.inspect(|c| {
        c.prepare("SELECT persona_id, session_id, text, source FROM memory_facts ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect()
    })
    .await
}

async fn run(e: &Env, s: i64, scenario: &str) -> String {
    e.say(s, "c1", &format!("#scn:{scenario} remember")).await;
    let (_, tool) = finished(e, s).await;
    tool.unwrap()["meta"]["code"].as_str().unwrap().to_string()
}

async fn reply_to(e: &Env, s: i64, id: &str) -> String {
    e.say(s, id, "#scn:context recall").await;
    let msgs = e.wait_agent_done(s, 1).await;
    msgs.iter().rev().find(|m| m["role"] == "agent").unwrap()["body_md"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_write_own_mode_stores_fact_with_persona_and_logs() {
    let mut e = Env::start().await;
    let p = persona(&e, "Anna", "own").await;
    let s = session_of(&e, p).await;
    assert_eq!(run(&e, s, "action_memory").await, "ok");
    let f = facts(&mut e).await;
    assert_eq!(
        f,
        [(
            Some(p),
            Some(s),
            "The user's favorite color is blue".to_string(),
            "agent".to_string()
        )]
    );
    let rows = common::agent_log(&mut e, "memory_write").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2.as_deref(), Some("ok"));
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_write_shared_mode_has_no_persona() {
    let mut e = Env::start().await;
    let p = persona(&e, "Shared", "shared").await;
    let s = session_of(&e, p).await;
    assert_eq!(run(&e, s, "action_memory").await, "ok");
    assert_eq!(
        facts(&mut e).await[0].0,
        None,
        "shared memory has no persona_id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_write_none_mode_does_not_store() {
    let mut e = Env::start().await;
    let p = persona(&e, "No memory", "none").await;
    let s = session_of(&e, p).await;
    assert_eq!(run(&e, s, "action_memory").await, "memory_disabled");
    assert!(facts(&mut e).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_write_rejects_empty_denied_and_flood() {
    let mut e = Env::start().await;
    let p = persona(&e, "Anna", "own").await;
    let s = session_of(&e, p).await;
    assert_eq!(run(&e, s, "action_memory_empty").await, "invalid_args");
    e.admin
        .put(
            "/permissions/global/0/memory_write",
            json!({"mode": "deny"}),
        )
        .await;
    let s2 = session_of(&e, p).await;
    assert_eq!(run(&e, s2, "action_memory").await, "permission_denied");
    assert!(
        facts(&mut e).await.is_empty(),
        "a denial and empty text write nothing"
    );
    e.admin
        .put(
            "/permissions/global/0/memory_write",
            json!({"mode": "allow"}),
        )
        .await;
    let s3 = session_of(&e, p).await;
    for i in 0..7 {
        e.say(s3, &format!("f{i}"), "#scn:action_memory remember")
            .await;
        finished(&e, s3).await;
    }
    let n = facts(&mut e).await.len();
    assert_eq!(n, 5, "at most five writes per minute per session");
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_foreign_persona_facts_are_not_in_context() {
    let e = Env::start().await;
    let a = persona(&e, "A", "own").await;
    let b = persona(&e, "B", "own").await;
    let sa = session_of(&e, a).await;
    assert_eq!(run(&e, sa, "action_memory").await, "ok");
    let sb = session_of(&e, b).await;
    let reply = reply_to(&e, sb, "c1").await;
    assert!(
        !reply.contains("favorite color"),
        "fact of another persona in context: {reply}"
    );
    let sa2 = session_of(&e, a).await;
    let reply = reply_to(&e, sa2, "c2").await;
    assert!(
        reply.contains("The user's favorite color is blue"),
        "own facts in context: {reply}"
    );
}
