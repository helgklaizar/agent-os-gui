//! A3-11: message texts, replies and keys do not reach the log; `X-Request-Id` is logged for send and stop.

mod common;

use common::{Env, raw_hdr};
use serde_json::json;

const TEXT: &str = "unique-message-text-7c1d";
const KEY: &str = "sk-logs-SECRET-5588";
const RID_SEND: &str = "0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a";
const RID_STOP: &str = "0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4b";

#[tokio::test(flavor = "multi_thread")]
async fn chat_text_and_keys_not_in_logs_request_id_present() {
    let e = Env::start_trace().await;
    let (acc, s) = e.model_session().await;
    e.admin
        .put(&format!("/secrets/account.{acc}"), json!({"value": KEY}))
        .await;
    let body = json!({"body_md": format!("#scn:hang {TEXT}"), "client_id": "log-1"}).to_string();
    let r = raw_hdr(
        &e.sock,
        &e.chat_token,
        "POST",
        &format!("/sessions/{s}/messages"),
        Some("application/json"),
        &[("X-Request-Id", RID_SEND)],
        body.as_bytes(),
    )
    .await;
    assert_eq!(r.status, 201);
    e.wait_agent_done_or_streaming(s).await;
    let r = raw_hdr(
        &e.sock,
        &e.chat_token,
        "POST",
        &format!("/sessions/{s}/stop"),
        Some("application/json"),
        &[("X-Request-Id", RID_STOP)],
        b"{}",
    )
    .await;
    assert_eq!(r.status, 204);
    e.wait_agent_done(s, 1).await;
    let log = std::fs::read_to_string(&e.log).unwrap();
    assert!(
        log.contains(RID_SEND) && log.contains(RID_STOP),
        "request_id of the send and stop requests is in the log"
    );
    assert!(
        log.contains("event=\"stop\"") || log.contains("event=stop"),
        "stop is logged"
    );
    assert!(!log.contains(TEXT), "message text in the log");
    assert!(!log.contains(KEY), "key in the log");
    assert!(!log.contains("Thinking"), "reply text in the log");
}
