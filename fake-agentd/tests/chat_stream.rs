//! Streaming the agent reply over WS `session.<id>`, stop, error, disconnect, injections.

mod chat_common;

use chat_common::*;
use serde_json::Value;

fn deltas(w: &Watch) -> Vec<Value> {
    w.events("message.delta")
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_send_and_stream_roundtrip() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some(),
        "snapshot"
    );
    let snap = w.snapshot();
    assert_eq!(snap["session"]["id"], s);
    assert!(
        snap["messages"].is_array() && snap["open_requests"].is_array() && snap["tasks"].is_array()
    );
    assert_eq!(r.say(s, "hello world", UUID_A).await.status, 201);
    let done = w.done(10).await;
    let done = done.unwrap_or_else(|| panic!("no message.done: {:?}", w.all()));
    assert_eq!(done["status"], "done");
    let created = w.events("message.created");
    assert_eq!(
        created[0]["message"]["role"], "user",
        "the user bubble first"
    );
    let agent = &created[1]["message"];
    assert_eq!(agent["role"], "agent");
    assert_eq!(agent["status"], "streaming");
    assert_eq!(agent["body_md"], "");
    assert!(agent["author"].as_str().unwrap_or("").starts_with("Agent"));
    let mut len = 0usize;
    let mut text = String::new();
    for d in deltas(&w) {
        assert_eq!(d["message_id"], agent["id"]);
        assert_eq!(
            d["offset"].as_u64(),
            Some(len as u64),
            "offset = number of bytes before the chunk"
        );
        let t = d["text"].as_str().unwrap_or("");
        len += t.len();
        text.push_str(t);
    }
    assert!(deltas(&w).len() >= 2, "the reply comes in chunks");
    assert_eq!(done["final_len"].as_u64(), Some(len as u64));
    assert!(text.contains("hello world"));
    assert!(!w.events("typing").is_empty(), "typing on/off");
    let row = r.log_rows(&format!(
        "SELECT status, body_md FROM messages WHERE role = 'agent' AND session_id = {s}"
    ));
    assert_eq!(row[0][0].as_deref(), Some("done"));
    assert_eq!(row[0][1].as_deref(), Some(text.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_stop_roundtrip() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/slow", UUID_A).await;
    assert!(
        w.until(10, |w| (deltas(w).len() >= 2).then_some(()))
            .await
            .is_some(),
        "the stream started"
    );
    let stop = r
        .call("POST", &format!("/v1/sessions/{s}/stop"), None)
        .await;
    assert_eq!(stop.status, 204);
    let done = w.done(10).await.unwrap_or_else(|| panic!("no done"));
    assert_eq!(done["status"], "stopped");
    let row = r.log_rows(&format!(
        "SELECT status, length(CAST(body_md AS BLOB)) FROM messages WHERE role = 'agent' AND session_id = {s}"
    ));
    assert_eq!(row[0][0].as_deref(), Some("stopped"));
    assert_ne!(row[0][1].as_deref(), Some("0"), "partial text saved");
    assert_eq!(
        row[0][1].as_deref(),
        Some(done["final_len"].to_string().as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_error_status() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/error", UUID_A).await;
    let done = w.done(10).await.unwrap_or_else(|| panic!("no done"));
    assert_eq!(done["status"], "error");
    let row = r.log_rows(&format!(
        "SELECT status FROM messages WHERE role = 'agent' AND session_id = {s}"
    ));
    assert_eq!(row[0][0].as_deref(), Some("error"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_injections_html_and_100k() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/html", UUID_A).await;
    w.done(10).await.unwrap_or_else(|| panic!("no done html"));
    let body = r
        .messages(s)
        .await
        .into_iter()
        .find(|m| m["role"] == "agent")
        .map(|m| m["body_md"].clone())
        .unwrap_or_default();
    assert!(
        body.as_str().unwrap_or("").contains("<script>"),
        "HTML arrives as is: the client escapes"
    );
    r.say(s, "/long", UUID_B).await;
    w.until(20, |w| (w.events("message.done").len() >= 2).then_some(()))
        .await
        .unwrap_or_else(|| panic!("no done long"));
    let long = r
        .messages(s)
        .await
        .into_iter()
        .find(|m| m["role"] == "agent" && m["body_md"].as_str().unwrap_or("").len() >= 100_000);
    assert!(long.is_some(), "a 100,000-character reply");
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_resubscribe_no_dup_by_offset() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    r.say(s, "/drop long reply for checking a connection drop", UUID_A)
        .await;
    for _ in 0..500 {
        if r.d.ws_connections() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(r.d.ws_connections(), 0, "WS dropped mid-stream");
    drop(w);
    r.d.restore();
    let w2 = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    w2.done(10)
        .await
        .unwrap_or_else(|| panic!("the stream finished after restore"));
    let snap = w2.snapshot();
    let partial = snap["messages"]
        .as_array()
        .and_then(|m| m.iter().find(|x| x["role"] == "agent"))
        .map(|m| m["body_md"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert!(
        !partial.is_empty(),
        "the snapshot carries the accumulated streaming text"
    );
    let mut text = partial.clone();
    let mut resync = 0;
    for d in deltas(&w2) {
        if d["offset"].as_u64() == Some(text.len() as u64) {
            text.push_str(d["text"].as_str().unwrap_or(""));
        } else {
            resync += 1;
        }
    }
    let full = r
        .messages(s)
        .await
        .into_iter()
        .find(|m| m["role"] == "agent")
        .map(|m| m["body_md"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert_eq!(text, full, "text without duplicates and gaps");
    assert!(
        resync >= 1,
        "the repeated delta arrives with a stale offset"
    );
}
