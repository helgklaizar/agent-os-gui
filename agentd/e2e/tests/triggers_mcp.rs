//! MCP as a trigger source (`source = mcp`, `match.from` is the server name) and the
//! trigger action "create a task run" (`task_id`).

mod common;

use std::time::Duration;

use common::{Env, ask, finished_runs, log_of, mk_task, setup};
use serde_json::{Value, json};

fn notifying(e: &Env) -> Value {
    json!({"env": {"MCP_FAKE_LOG": log_of(e).to_str().unwrap(), "MCP_FAKE_NOTIFY": "1"}})
}

async fn session_of_model(e: &Env) -> i64 {
    e.model_session().await.1
}

async fn trigger(e: &Env, body: Value) -> Value {
    let (st, v) = e.admin.post("/triggers", body).await;
    assert_eq!(st, 201, "{v}");
    v
}

async fn wait_msgs(e: &Env, s: i64, role: &str) -> Vec<Value> {
    for _ in 0..200 {
        let m: Vec<Value> = e
            .messages(s)
            .await
            .into_iter()
            .filter(|m| m["role"] == role)
            .collect();
        if !m.is_empty() {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_notification_fires_trigger_with_untrusted_context() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), notifying(&e)).await;
    let caller = session_of_model(&e).await;
    let target = session_of_model(&e).await;
    let t = trigger(
        &e,
        json!({"source": "mcp", "match": {"from": "fake", "contains": "alarm"}, "session_id": target}),
    )
    .await;
    assert_eq!(t["source"], "mcp");
    ask(&e, caller, "action_mcp_alarm").await;
    let tool = wait_msgs(&e, target, "tool").await;
    assert_eq!(
        tool.len(),
        1,
        "notification delivered to the trigger session"
    );
    let body = tool[0]["body_md"].as_str().unwrap();
    for must in ["alarm: disk 95%", "\"untrusted\":true", "fake"] {
        assert!(body.contains(must), "{must} in {body}");
    }
    assert!(
        !wait_msgs(&e, target, "user").await.is_empty(),
        "agent turn started"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_notification_from_other_server_does_not_fire() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), notifying(&e)).await;
    let caller = session_of_model(&e).await;
    let target = session_of_model(&e).await;
    trigger(
        &e,
        json!({"source": "mcp", "match": {"from": "other"}, "session_id": target}),
    )
    .await;
    trigger(
        &e,
        json!({"source": "mcp", "match": {"contains": "quiet"}, "session_id": target}),
    )
    .await;
    ask(&e, caller, "action_mcp_alarm").await;
    let _ = e.wait_agent_done(caller, 2).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        e.messages(target).await.is_empty(),
        "a foreign server name and foreign text do not fire"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_creates_task_run() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let task = mk_task(&e, s, common::deleg("check the inbox")).await["id"]
        .as_i64()
        .unwrap();
    let acc = e
        .admin
        .make("/accounts", json!({"type": "tg", "label": "tg"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let t = trigger(
        &e,
        json!({"source": "tg", "match": {"contains": "run"}, "session_id": s, "task_id": task}),
    )
    .await;
    assert_eq!(t["task_id"], task, "{t}");
    let (st, v) = e
        .admin
        .post(
            "/_test/ingest",
            json!({"account_id": acc, "ext_id": "m1", "handle": "@a", "body": "run the task"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let runs = finished_runs(&e, task, 1).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_task_id_is_validated() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    for bad in [json!(0), json!("x"), json!(99999)] {
        let (st, v) = e
            .admin
            .post(
                "/triggers",
                json!({"source": "tg", "match": {}, "session_id": s, "task_id": bad}),
            )
            .await;
        assert_eq!(st, 422, "{bad}: {v}");
        assert!(common::fields(&v).contains(&"task_id".to_string()), "{v}");
    }
}

/// Two MCP notifications on one trigger give two turns (the turn `client_id` is unique).
#[tokio::test(flavor = "multi_thread")]
async fn repeated_mcp_notification_starts_a_new_turn() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), notifying(&e)).await;
    let caller = session_of_model(&e).await;
    let target = session_of_model(&e).await;
    trigger(
        &e,
        json!({"source": "mcp", "match": {"from": "fake", "contains": "alarm"}, "session_id": target}),
    )
    .await;
    ask(&e, caller, "action_mcp_alarm").await;
    assert_eq!(wait_msgs(&e, target, "user").await.len(), 1);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    e.say(caller, "c2", "#scn:action_mcp_alarm call").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let users: Vec<Value> = e
        .messages(target)
        .await
        .into_iter()
        .filter(|m| m["role"] == "user")
        .collect();
    assert_eq!(
        users.len(),
        2,
        "the second notification did not start a turn: {users:?}"
    );
}

/// Same for a trigger with `task_id`: the second notification runs the task again.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_mcp_notification_runs_the_task_again() {
    let e = Env::start().await;
    setup(&e, json!({"echo": "read"}), notifying(&e)).await;
    let caller = session_of_model(&e).await;
    let target = session_of_model(&e).await;
    let task = mk_task(&e, target, common::deleg("check the notification")).await["id"]
        .as_i64()
        .unwrap();
    trigger(
        &e,
        json!({"source": "mcp", "match": {"from": "fake", "contains": "alarm"}, "session_id": target, "task_id": task}),
    )
    .await;
    ask(&e, caller, "action_mcp_alarm").await;
    assert!(!finished_runs(&e, task, 1).await.is_empty());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    e.say(caller, "c2", "#scn:action_mcp_alarm call").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let runs = common::runs(&e, task).await;
    assert_eq!(
        runs.len(),
        2,
        "the second notification did not start the task: {runs:?}"
    );
}
