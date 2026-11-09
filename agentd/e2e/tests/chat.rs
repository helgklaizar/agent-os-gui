//! A3-08: reply stream, stop, errors, queue, restart, on the real `agentd` binary with the `scripted` provider.

mod common;

use std::time::Duration;

use common::{Env, is_done, kind_of};
use core_client::api::{Frame, FrameKind};
use serde_json::{Value, json};

const LONG: Duration = Duration::from_secs(20);

fn deltas(frames: &[Frame]) -> Vec<&Value> {
    frames
        .iter()
        .filter(|f| kind_of(f) == "message.delta")
        .map(|f| &f.data)
        .collect()
}

fn done_of(frames: &[Frame]) -> &Value {
    &frames
        .iter()
        .find(|f| is_done(f))
        .expect("message.done")
        .data
}

/// Client-side `offset` rule: apply only if `offset == length of own text` (api.md 4.4).
fn assemble(frames: &[Frame]) -> String {
    let mut text = String::new();
    for d in deltas(frames) {
        assert_eq!(
            d["offset"].as_u64().unwrap() as usize,
            text.len(),
            "delta offset: {d}"
        );
        text.push_str(d["text"].as_str().unwrap());
    }
    text
}

fn agent(msgs: &[Value]) -> Vec<&Value> {
    msgs.iter().filter(|m| m["role"] == "agent").collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_chat_send_and_stream_roundtrip() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    let snap = ws.subscribe(&format!("session.{s}"));
    assert_eq!(snap["messages"].as_array().unwrap().len(), 0);
    assert_eq!(snap["session"]["id"], s);
    assert!(
        snap["open_requests"].as_array().unwrap().is_empty()
            && snap["tasks"].as_array().unwrap().is_empty()
    );
    let (st, user) = e.say(s, "c1", "hello").await;
    assert_eq!(st, 201, "{user}");
    let frames = ws.until(LONG, is_done);
    let kinds: Vec<&str> = frames.iter().map(kind_of).collect();
    assert_eq!(kinds[0], "message.created", "{kinds:?}");
    assert_eq!(frames[0].data["message"]["role"], "user");
    assert_eq!(frames[1].data["message"]["role"], "agent");
    assert_eq!(frames[1].data["message"]["status"], "streaming");
    assert!(
        kinds.contains(&"typing") && !deltas(&frames).is_empty(),
        "{kinds:?}"
    );
    assert!(frames.iter().all(|f| f.kind == FrameKind::Event));
    let text = assemble(&frames);
    assert_eq!(text, "Reply: hello");
    let done = done_of(&frames);
    assert_eq!(
        (done["status"].as_str(), done["final_len"].as_u64()),
        (Some("done"), Some(text.len() as u64))
    );
    let all = e.messages(s).await;
    assert_eq!(all.len(), 2);
    assert_eq!(
        (all[0]["role"].as_str(), all[1]["role"].as_str()),
        (Some("user"), Some("agent"))
    );
    assert_eq!(all[1]["body_md"], text.as_str());
    assert_eq!(all[1]["author"], "Agent");
    assert_eq!(all[1]["client_id"], Value::Null);
    assert!(
        all[1]["meta"]["usage"]["tokens_in"].as_u64().unwrap() > 0,
        "usage in meta"
    );
    assert_eq!(all[1]["id"], done["message_id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_100k_chars_and_html_verbatim() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "long", "#scn:long").await;
    let frames = ws.until(LONG, is_done);
    let text = assemble(&frames);
    assert_eq!(text.chars().count(), 100_000);
    assert_eq!(
        done_of(&frames)["final_len"].as_u64().unwrap() as usize,
        text.len()
    );
    let m = e.messages(s).await;
    assert_eq!(
        agent(&m)[0]["body_md"].as_str().unwrap().len(),
        text.len(),
        "whole in the DB"
    );
    e.say(s, "html", "#scn:html").await;
    let frames = ws.until(LONG, is_done);
    let body = assemble(&frames);
    assert!(
        body.contains("<script>alert(1)</script>") && body.contains("**bold**"),
        "{body}"
    );
    let m = e.messages(s).await;
    assert_eq!(
        agent(&m)[1]["body_md"],
        body.as_str(),
        "HTML is stored as is"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_stop_keeps_partial() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "h", "#scn:hang").await;
    ws.until(LONG, |f| kind_of(f) == "message.delta");
    let (st, _) = e.chat.post(&format!("/sessions/{s}/stop"), json!({})).await;
    assert_eq!(st, 204);
    let frames = ws.until(LONG, is_done);
    let done = done_of(&frames);
    assert_eq!(done["status"], "stopped");
    let m = e.messages(s).await;
    let a = agent(&m)[0];
    assert_eq!(
        (a["status"].as_str(), a["body_md"].as_str()),
        (Some("stopped"), Some("Thinking… "))
    );
    assert_eq!(
        done["final_len"].as_u64().unwrap() as usize,
        a["body_md"].as_str().unwrap().len()
    );
    let (st, _) = e.chat.post(&format!("/sessions/{s}/stop"), json!({})).await;
    assert_eq!(st, 204, "repeated stop with no reply: no error");
    let (st, _) = e.chat.post("/sessions/9999/stop", json!({})).await;
    assert_eq!(st, 404);
    let (st, _) = e.read.post(&format!("/sessions/{s}/stop"), json!({})).await;
    assert_eq!(st, 403);
    // After a stop the session replies again.
    e.say(s, "again", "more").await;
    let m = e.wait_agent_done(s, 2).await;
    assert_eq!(agent(&m)[1]["status"], "done");
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_provider_error_card() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "d", "#scn:drop").await;
    e.say(s, "r", "#scn:rate_limited").await;
    let m = e.wait_agent_done(s, 2).await;
    let a = agent(&m);
    assert_eq!(a[0]["status"], "error");
    assert_eq!(
        a[0]["meta"]["error"],
        json!({"code": "network", "retryable": true})
    );
    assert_eq!(
        a[0]["body_md"], "Beginning of the answer, ",
        "partial text is kept"
    );
    assert_eq!(a[1]["status"], "error");
    assert_eq!(a[1]["meta"]["error"]["code"], "rate_limited");
    assert_eq!(a[1]["body_md"], "");
    assert!(
        !m.to_vec().iter().any(|x| x.to_string().contains("sk-")),
        "no trace of keys"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_model_error_message() {
    let e = Env::start().await;
    let s = e.session("no model").await;
    let (st, _) = e.say(s, "c", "hello").await;
    assert_eq!(st, 201);
    let m = e.wait_agent_done(s, 1).await;
    let a = agent(&m)[0];
    assert_eq!(
        (
            a["status"].as_str(),
            a["meta"]["error"]["code"].as_str(),
            a["body_md"].as_str()
        ),
        (Some("error"), Some("no_model"), Some(""))
    );
    assert_eq!(a["meta"]["error"]["retryable"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_queue_second_message() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let (_, u1) = e.say(s, "q1", "#scn:slow first").await;
    let (_, u2) = e.say(s, "q2", "second").await;
    let frames = ws.until(Duration::from_secs(30), {
        let mut dones = 0;
        move |f| {
            if is_done(f) {
                dones += 1;
            }
            dones == 2
        }
    });
    let order: Vec<(&str, i64)> = frames
        .iter()
        .filter(|f| matches!(kind_of(f), "message.created" | "message.done"))
        .map(|f| {
            (
                kind_of(f),
                f.data["message"]["id"]
                    .as_i64()
                    .or(f.data["message_id"].as_i64())
                    .unwrap(),
            )
        })
        .collect();
    let m = e.messages(s).await;
    let ids: Vec<i64> = m.iter().map(|x| x["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    let (a1, a2) = (
        agent(&m)[0]["id"].as_i64().unwrap(),
        agent(&m)[1]["id"].as_i64().unwrap(),
    );
    let pos = |k: &str, id: i64| {
        order
            .iter()
            .position(|o| *o == (k, id))
            .unwrap_or_else(|| panic!("{k} {id}: {order:?}"))
    };
    assert!(
        pos("message.done", a1) < pos("message.created", a2),
        "second reply after the first: {order:?}"
    );
    assert!(
        m.iter().position(|x| x["id"] == u1["id"]) < m.iter().position(|x| x["id"] == u2["id"])
    );
    let bodies: Vec<&str> = agent(&m)
        .iter()
        .map(|x| x["body_md"].as_str().unwrap())
        .collect();
    assert_eq!(
        bodies,
        ["Reply: #scn:slow first", "Reply: second"],
        "replies follow message order"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_queue_limit_429() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let (st, _) = e.say(s, "q0", "#scn:hang").await;
    assert_eq!(st, 201);
    for i in 1..=10 {
        let (st, b) = e.say(s, &format!("q{i}"), "to the queue").await;
        assert_eq!(st, 201, "{i}: {b}");
    }
    let (st, b) = e.say(s, "q11", "extra").await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (429, Some("rate_limited"))
    );
    assert!(b["error"]["retryable"].as_bool().unwrap());
    let users = e
        .messages(s)
        .await
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    assert_eq!(users, 11, "rejected message is not stored");
}
