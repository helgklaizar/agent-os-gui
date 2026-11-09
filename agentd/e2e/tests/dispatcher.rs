//! A4-06: agent action dispatcher: tool call -> gate -> permission request -> observer -> executor -> model.
//! The `scripted` provider requests an action (`#scn:action_*`); the core is real.

mod common;

use common::{Env, agent_log, finished, row};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_ask_allow_deny_expired() {
    let mut e = Env::start_clock().await;
    let (_, s) = e.model_session().await;
    // allow: card -> decision -> executor (no connector) -> the model got the result.
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "send");
    assert_eq!(e.decide(id, json!({"decision": "allow"})).await.0, 200);
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("no_connector"), "{agent:?}");
    let tool = tool.unwrap();
    assert_eq!(
        (tool["meta"]["ok"].as_bool(), tool["meta"]["code"].as_str()),
        (Some(false), Some("no_connector")),
        "{tool}"
    );
    // deny: the model receives the `denied` tool error.
    let (_, s2) = e.model_session().await;
    e.say(s2, "c2", "#scn:action_send more").await;
    let (id2, _) = e.wait_card(s2).await;
    assert_eq!(e.decide(id2, json!({"decision": "deny"})).await.0, 200);
    let (agent, tool) = finished(&e, s2).await;
    assert!(agent[1].contains("denied"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "denied");
    // expired: clock +10 minutes, action not executed.
    let (_, s3) = e.model_session().await;
    e.say(s3, "c3", "#scn:action_pay invoice").await;
    let (id3, _) = e.wait_card(s3).await;
    e.advance(601).await;
    let (agent, tool) = finished(&e, s3).await;
    assert!(agent[1].contains("expired"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "expired");
    let sends = agent_log(&mut e, "send").await;
    assert_eq!(
        sends,
        vec![
            row("send", "ask-approved", "no_connector"),
            row("send", "ask-denied", "denied")
        ]
    );
    let pays = agent_log(&mut e, "pay").await;
    assert_eq!(pays, vec![row("pay", "deny", "expired")]);
    let _ = id3;
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_allow_logs_decision_allow() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/send", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_send hello").await;
    let (agent, _) = finished(&e, s).await;
    assert!(agent[1].contains("no_connector"), "{agent:?}");
    assert!(
        e.messages(s).await.iter().all(|m| m["role"] != "system"),
        "no card"
    );
    assert_eq!(
        agent_log(&mut e, "send").await,
        vec![row("send", "allow", "no_connector")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_executor_unavailable_fail_closed() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_shell ls").await;
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("executor_unavailable"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "executor_unavailable");
    assert_eq!(
        agent_log(&mut e, "shell").await,
        vec![row("shell", "allow", "executor_unavailable")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_policy_deny_returns_error() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/pay", json!({"mode": "deny"}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_pay invoice").await;
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("permission_denied"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "permission_denied");
    assert_eq!(
        agent_log(&mut e, "pay").await,
        vec![row("pay", "deny", "policy_deny")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_config_write_goes_through_settings() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put(
                "/permissions/global/0/config_write",
                json!({"mode": "allow"})
            )
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_config theme").await;
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("\"ok\":true"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["ok"], true);
    let (_, theme) = e.read.get("/settings/theme").await;
    assert_eq!(theme["theme"], "light", "setting applied by the executor");
    assert_eq!(
        agent_log(&mut e, "config_write").await,
        vec![row("config_write", "allow", "ok")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_open_request() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    let (st, _) = e.chat.post(&format!("/sessions/{s}/stop"), json!({})).await;
    assert_eq!(st, 204);
    for _ in 0..100 {
        let (_, r) = e.read.get(&format!("/permission-requests/{id}")).await;
        if r["status"] == "denied" {
            break;
        }
        common::pause(50).await;
    }
    let (_, r) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(r["status"], "denied");
    assert_eq!(
        agent_log(&mut e, "send").await,
        vec![row("send", "deny", "cancelled")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_token_cannot_decide_or_change_permissions() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    for api in [&e.agent, &e.module] {
        let (st, _) = api
            .post(
                &format!("/permission-requests/{id}/decision"),
                json!({"decision": "allow"}),
            )
            .await;
        assert_eq!(st, 403);
        let (st, _) = api
            .put("/permissions/global/0/send", json!({"mode": "allow"}))
            .await;
        assert_eq!(st, 403);
    }
    let (_, r) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(r["status"], "pending");
}
