//! A5-fix Z-1: `task_runs.output` stores the agent reply (with a limit), not only the error reason.

mod common;

use common::{Env, deleg, finished_runs, mk_task};
use serde_json::json;

/// Limit on the stored run output length, bytes.
const OUTPUT_MAX: usize = 4000;

async fn agent_text(e: &Env, s: i64) -> String {
    e.messages(s)
        .await
        .iter()
        .filter(|m| m["role"] == "agent")
        .map(|m| m["body_md"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test(flavor = "multi_thread")]
async fn task_run_output_keeps_agent_reply() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, deleg("#scn:chunks make the report")).await;
    let id = t["id"].as_i64().unwrap();
    let (st, _) = e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    assert_eq!(st, 202);
    let done = finished_runs(&e, id, 1).await;
    assert_eq!(done[0]["status"], "ok", "{done:?}");
    let reply = agent_text(&e, s).await;
    assert!(!reply.is_empty(), "the agent replied");
    assert_eq!(done[0]["output"].as_str(), Some(reply.as_str()), "{done:?}");
}

/// The agent reply is `Reply: <spec>`; ASCII padding puts a two-byte character at bytes `OUTPUT_MAX-1..OUTPUT_MAX+1`,
/// so slicing at the limit lands inside the character and, without moving back to the boundary, yields invalid UTF-8.
#[tokio::test(flavor = "multi_thread")]
async fn task_run_output_is_capped_on_char_boundary() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let head = "Reply: #scn:chunks ";
    let long = format!(
        "{}{}",
        "a".repeat(OUTPUT_MAX - 1 - head.len()),
        "é".repeat(10)
    );
    let t = mk_task(&e, s, deleg(&format!("#scn:chunks {long}"))).await;
    let id = t["id"].as_i64().unwrap();
    let (st, _) = e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    assert_eq!(st, 202);
    let done = finished_runs(&e, id, 1).await;
    let out = done[0]["output"].as_str().unwrap_or_default();
    let reply = agent_text(&e, s).await;
    assert_eq!(
        reply.as_bytes()[OUTPUT_MAX - 1],
        "é".as_bytes()[0],
        "the character starts at byte limit-1"
    );
    assert_eq!(
        out.len(),
        OUTPUT_MAX - 1,
        "the slice backed off to the character boundary"
    );
    assert!(
        reply.starts_with(out),
        "the output is the beginning of the reply"
    );
}
