//! A3-08: WS drop, resubscribe, core restart, unread counter, reply context, on the real `agentd` binary with `scripted`.

mod common;

use std::time::Duration;

use common::{Env, Ws, is_done, kind_of};
use core_client::api::Frame;
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

fn agent(msgs: &[Value]) -> Vec<&Value> {
    msgs.iter().filter(|m| m["role"] == "agent").collect()
}

fn replay(snapshot_body: &str, frames: &[Frame]) -> String {
    let mut text = snapshot_body.to_string();
    for d in deltas(frames) {
        let (off, t) = (
            d["offset"].as_u64().unwrap() as usize,
            d["text"].as_str().unwrap(),
        );
        if off == text.len() {
            text.push_str(t);
        } else {
            assert!(
                off + t.len() <= text.len(),
                "gap: offset {off}, length {}",
                text.len()
            );
        }
    }
    text
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_chat_survives_ws_drop_mid_stream() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "t", "#scn:trickle").await;
    ws.until(LONG, |f| kind_of(f) == "message.delta");
    drop(ws);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mut ws2 = e.ws();
    let snap = ws2.subscribe(&format!("session.{s}"));
    let msgs = snap["messages"].as_array().unwrap();
    let a = msgs.iter().find(|m| m["role"] == "agent").unwrap();
    assert_eq!(
        a["status"], "streaming",
        "snapshot returns the accumulated stream text"
    );
    assert!(!a["body_md"].as_str().unwrap().is_empty());
    let frames = ws2.until(LONG, is_done);
    let text = replay(a["body_md"].as_str().unwrap(), &frames);
    let m = e.messages(s).await;
    assert_eq!(
        agent(&m)[0]["body_md"],
        text.as_str(),
        "no duplicates and no gaps"
    );
    assert_eq!(
        done_of(&frames)["final_len"].as_u64().unwrap() as usize,
        text.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_resubscribe_no_dup_by_offset() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    let topic = format!("session.{s}");
    ws.subscribe(&topic);
    e.say(s, "t", "#scn:trickle").await;
    ws.until(LONG, |f| kind_of(f) == "message.delta");
    ws.client.unsubscribe(&topic).unwrap();
    let snap = ws.subscribe(&topic);
    let a = snap["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "agent")
        .unwrap()
        .clone();
    let frames = ws.until(LONG, is_done);
    let text = replay(a["body_md"].as_str().unwrap(), &frames);
    assert_eq!(agent(&e.messages(s).await)[0]["body_md"], text.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_chat_queue_no_duplicates() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let ws: Ws = e.ws();
    drop(ws);
    for i in 0..3 {
        let (st, _) = e.say(s, &format!("k{i}"), &format!("message {i}")).await;
        assert_eq!(st, 201);
    }
    // Outgoing queue replay after a drop: same client_id values.
    for i in 0..3 {
        let (st, _) = e.say(s, &format!("k{i}"), &format!("message {i}")).await;
        assert_eq!(st, 200, "replay {i}");
    }
    let m = e.wait_agent_done(s, 3).await;
    let users: Vec<&str> = m
        .iter()
        .filter(|x| x["role"] == "user")
        .map(|x| x["body_md"].as_str().unwrap())
        .collect();
    assert_eq!(users, ["message 0", "message 1", "message 2"]);
    assert_eq!(agent(&m).len(), 3, "exactly three replies");
    let mut ws = e.ws();
    let snap = ws.subscribe(&format!("session.{s}"));
    assert_eq!(snap["messages"].as_array().unwrap().len(), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_marks_streaming_interrupted() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "h", "#scn:hang").await;
    ws.until(LONG, |f| kind_of(f) == "message.delta");
    drop(ws);
    e.crash_and_restart();
    let m = e.messages(s).await;
    let a = agent(&m)[0];
    assert_eq!(a["status"], "interrupted");
    assert_eq!(a["body_md"], "Thinking… ", "accumulated text preserved");
    let mut ws = e.ws();
    let snap = ws.subscribe(&format!("session.{s}"));
    let sa = snap["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "agent")
        .unwrap()
        .clone();
    assert_eq!(
        sa["status"], "interrupted",
        "snapshot returns the final state"
    );
    let left: i64 = e
        .inspect(|c| {
            c.query_row(
                "SELECT count(*) FROM messages WHERE status = 'streaming'",
                [],
                |r| r.get(0),
            )
        })
        .await;
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn message_unread_bump_when_not_watched() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut tree = e.ws();
    tree.subscribe("sessions");
    e.say(s, "u", "hello").await;
    e.wait_agent_done(s, 1).await;
    let frames = tree.drain(Duration::from_millis(500));
    let unread: Vec<&Frame> = frames.iter().filter(|f| f.data["op"] == "unread").collect();
    assert_eq!(unread.len(), 1, "{frames:?}");
    assert_eq!(
        (
            unread[0].data["session_id"].as_i64(),
            unread[0].data["unread"].as_i64()
        ),
        (Some(s), Some(1))
    );
    let (_, sess) = e.read.get(&format!("/sessions/{s}")).await;
    assert_eq!(sess["unread"], 1);
    // An open session (has a session.<id> subscriber) does not raise the badge.
    let mut watcher = e.ws();
    watcher.subscribe(&format!("session.{s}"));
    e.say(s, "u2", "more").await;
    watcher.until(LONG, is_done);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, sess) = e.read.get(&format!("/sessions/{s}")).await;
    assert_eq!(
        sess["unread"], 1,
        "counter did not grow while the session is open"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_system_prompt_history_and_budget() {
    let e = Env::start().await;
    std::fs::create_dir_all(e.home.join(".agentos")).unwrap();
    std::fs::write(
        e.home.join(".agentos/AGENT.md"),
        "You are a project assistant.",
    )
    .unwrap();
    let (acc, _) = e.model_session().await;
    let p = e
        .admin
        .make("/personas", json!({"name": "Bot", "color": "#fff", "model_acc": acc, "style": "brief", "lang": "ru", "signature": "— Bot"}))
        .await;
    let s = e
        .chat
        .make(
            "/sessions",
            json!({"name": "with persona", "persona_id": p["id"]}),
        )
        .await["id"]
        .as_i64()
        .unwrap();
    e.say(s, "a", "first").await;
    e.wait_agent_done(s, 1).await;
    e.say(s, "b", "#scn:context second").await;
    let m = e.wait_agent_done(s, 2).await;
    let a = agent(&m);
    assert_eq!(a[0]["author"], "Agent · Bot");
    let body = a[1]["body_md"].as_str().unwrap();
    assert!(
        body.starts_with(
            "SYS[You are a project assistant.\n\nReply style: brief\nReply language: ru\nSignature: — Bot]"
        ),
        "{body}"
    );
    assert!(
        body.contains("TURNS[u:first|a:Reply: first|u:#scn:context second]"),
        "{body}"
    );
    // A thread with ctx = parent sees the parent's tail.
    let t = e
        .chat
        .make(&format!("/sessions/{s}/threads"), json!({"name": "thread"}))
        .await["id"]
        .as_i64()
        .unwrap();
    e.say(t, "t1", "#scn:context in thread").await;
    let tm = e.wait_agent_done(t, 1).await;
    let tb = agent(&tm)[0]["body_md"].as_str().unwrap().to_string();
    assert!(
        tb.contains("u:first") && tb.contains("u:#scn:context in thread"),
        "{tb}"
    );
    // Context budget: 256 tokens = 1 KiB; a long message does not fit and is left out of the request.
    e.say(s, "big", &"x".repeat(3000)).await;
    e.wait_agent_done(s, 3).await;
    let (_, page) = e.admin.get("/settings/models").await;
    let (st, b) = e
        .admin
        .patch_if_match(
            "/settings/models",
            json!({"context_budget": 256}),
            page["rev"].as_i64().unwrap(),
        )
        .await;
    assert_eq!(st, 200, "{b}");
    e.say(s, "c", "#scn:context third").await;
    let m = e.wait_agent_done(s, 4).await;
    let last = agent(&m)[3]["body_md"].as_str().unwrap().to_string();
    assert!(last.contains("TURNS[u:#scn:context third]"), "{last}");
}
