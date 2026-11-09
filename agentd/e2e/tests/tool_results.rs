//! A4-07: `browser.command` and `POST /sessions/{id}/tool-results` — command owner, idempotency, limits, timeout.

mod common;

use std::time::Duration;

use common::{Env, Ws};
use serde_json::{Value, json};

/// The user requests the `browse` action; the GUI (a `session.<s>` subscriber with token `token`) receives the command.
async fn start_browse(e: &Env, token: &str) -> (i64, Ws, String) {
    let (_, s) = e.model_session().await;
    let mut ws = e.ws_as(token);
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_browse open").await;
    let frames = ws.until(Duration::from_secs(10), |f| {
        f.data["kind"] == "browser.command"
    });
    let cmd = frames.last().unwrap().data.clone();
    assert_eq!(cmd["command"], "navigate", "{cmd}");
    assert_eq!(cmd["url"], "https://example.test/");
    let rid = cmd["request_id"].as_str().unwrap().to_string();
    assert!(!rid.is_empty());
    (s, ws, rid)
}

fn result(rid: &str, output: &str) -> Value {
    json!({"request_id": rid, "kind": "browser", "status": "ok", "output": output,
           "content_type": "text/html", "url": "https://example.test/", "http_status": 200})
}

async fn post(api: &common::Api, s: i64, body: Value) -> common::Reply {
    api.post(&format!("/sessions/{s}/tool-results"), body).await
}

async fn tool_message(e: &Env, s: i64) -> Value {
    let m = e.wait_agent_done(s, 2).await;
    m.into_iter()
        .find(|x| x["role"] == "tool")
        .expect("tool message")
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_accepted_once() {
    let e = Env::start().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    let (st, v) = post(&e.chat, s, result(&rid, "<html>page</html>")).await;
    assert_eq!(
        (st, v.clone()),
        (202, json!({"request_id": rid, "status": "accepted"}))
    );
    let (st, v) = post(&e.chat, s, result(&rid, "<html>page</html>")).await;
    assert_eq!(
        (st, v["status"].as_str()),
        (202, Some("accepted")),
        "repeat of the same body"
    );
    // The model received the result; the turn is complete.
    let tool = tool_message(&e, s).await;
    assert_eq!(
        (tool["meta"]["ok"].as_bool(), tool["meta"]["code"].as_str()),
        (Some(true), Some("ok")),
        "{tool}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_conflict() {
    let e = Env::start().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    assert_eq!(post(&e.chat, s, result(&rid, "first")).await.0, 202);
    let (st, v) = post(&e.chat, s, result(&rid, "other")).await;
    assert_eq!(
        (st, v["error"]["details"]["reason"].as_str()),
        (409, Some("result_exists")),
        "{v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_foreign_token_403() {
    let e = Env::start().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    // Those who did not receive the command: another `admin`, the agent, a module.
    for (who, api) in [
        ("admin", &e.admin2),
        ("agent", &e.agent),
        ("module", &e.module),
    ] {
        let (st, v) = post(api, s, result(&rid, "forged")).await;
        assert_eq!(
            (st, v["error"]["code"].as_str()),
            (403, Some("permission_denied")),
            "{who}: {v}"
        );
    }
    // The command stayed open: the owner replies and the first genuine result is accepted.
    assert_eq!(post(&e.chat, s, result(&rid, "genuine")).await.0, 202);
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_unknown_and_invalid() {
    let e = Env::start().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    let (st, _) = post(&e.chat, s, result("no-such-id", "x")).await;
    assert_eq!(st, 404);
    let (other, _) = (
        e.chat.make("/sessions", json!({"name": "x"})).await["id"]
            .as_i64()
            .unwrap(),
        0,
    );
    let (st, _) = post(&e.chat, other, result(&rid, "x")).await;
    assert_eq!(st, 404, "the command belongs to another session");
    let (st, _) = post(&e.chat, 999_999, result(&rid, "x")).await;
    assert_eq!(st, 404);
    for bad in [
        json!({}),
        json!({"request_id": rid, "kind": "browser"}),
        json!({"request_id": rid, "kind": "tty", "status": "ok"}),
        json!({"request_id": rid, "kind": "browser", "status": "great"}),
        json!({"request_id": rid, "kind": "user", "status": "ok"}),
        json!({"request_id": rid, "kind": "browser", "status": "ok", "extra": 1}),
        json!({"request_id": rid, "kind": "browser", "status": "ok", "http_status": "200"}),
    ] {
        let (st, v) = post(&e.chat, s, bad.clone()).await;
        assert_eq!(st, 422, "{bad}: {v}");
    }
    assert_eq!(
        post(&e.chat, s, result(&rid, "ok")).await.0,
        202,
        "nothing invalid closed the command"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_size_limits() {
    let e = Env::start().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    // A body over 1 MiB — 413, the command stays open.
    let huge = "x".repeat(1024 * 1024 + 1);
    let (st, v) = post(&e.chat, s, result(&rid, &huge)).await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (413, Some("payload_too_large")),
        "{v}"
    );
    // Output of 256 KiB + 1 is accepted with a "truncated" mark.
    let big = "é".repeat(131_073);
    let (st, v) = post(&e.chat, s, result(&rid, &big)).await;
    assert_eq!(st, 202, "{v}");
    let tool = tool_message(&e, s).await;
    assert_eq!(tool["meta"]["truncated"], true, "{tool}");
    assert_eq!(tool["meta"]["output_bytes"], 262_146);
    assert!(
        tool["body_md"].as_str().unwrap().len() < 32 * 1024,
        "the model gets a short excerpt"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_command_timeout() {
    let e = Env::start_clock().await;
    let token = e.chat_token.clone();
    let (s, _ws, rid) = start_browse(&e, &token).await;
    e.advance(61).await;
    let tool = tool_message(&e, s).await;
    assert_eq!(
        (tool["meta"]["ok"].as_bool(), tool["meta"]["code"].as_str()),
        (Some(false), Some("timeout")),
        "{tool}"
    );
    let (st, v) = post(&e.chat, s, result(&rid, "late")).await;
    assert_eq!(
        (st, v["error"]["details"]["reason"].as_str()),
        (409, Some("request_closed")),
        "{v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_without_subscriber_fails_fast() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_browse open").await;
    let tool = tool_message(&e, s).await;
    assert_eq!(tool["meta"]["code"], "no_browser_client", "{tool}");
}
