//! S4: chat: idempotency, WS drop, queue, personas, session deletion, secrets kept out of logs.

use std::time::Duration;

use serde_json::json;

use crate::common::{self, Env, is_done, kind_of};

#[tokio::test(flavor = "multi_thread")]
async fn send_duplicate_client_id_idempotent() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let (st, first) = e.say(s, "same", "hello").await;
    assert_eq!(st, 201);
    let (st, again) = e.say(s, "same", "hello").await;
    assert_eq!((st, &again["id"]), (200, &first["id"]));
    let msgs = e.wait_agent_done(s, 1).await;
    assert_eq!(msgs.iter().filter(|m| m["role"] == "user").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_survives_ws_drop_mid_stream() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let topic = format!("session.{s}");
    let mut ws = e.ws();
    ws.subscribe(&topic);
    e.say(s, "c1", "#scn:trickle long").await;
    ws.until(Duration::from_secs(10), |f| kind_of(f) == "message.delta");
    drop(ws); // drop in the middle of the stream
    let mut ws = e.ws();
    let snap = ws.subscribe(&topic);
    let agent = snap["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "agent")
        .cloned()
        .unwrap();
    let mut text = agent["body_md"].as_str().unwrap().to_string();
    let frames = ws.until(Duration::from_secs(15), is_done);
    let mut final_len = 0;
    for f in &frames {
        match kind_of(f) {
            "message.delta" => {
                let off = f.data["offset"].as_u64().unwrap() as usize;
                assert!(
                    off <= text.len(),
                    "gap in text: offset {off} at {}",
                    text.len()
                );
                if off == text.len() {
                    text.push_str(f.data["text"].as_str().unwrap());
                }
            }
            "message.done" => final_len = f.data["final_len"].as_u64().unwrap() as usize,
            _ => {}
        }
    }
    let stored = e.wait_agent_done(s, 1).await;
    let body = stored.iter().find(|m| m["role"] == "agent").unwrap()["body_md"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        (text.len(), &text),
        (final_len, &body),
        "no duplicates or gaps"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_queue_no_duplicates() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    for round in 0..2 {
        for (i, cid) in ["q1", "q2", "q3"].iter().enumerate() {
            let (st, _) = e.say(s, cid, &format!("message {i}")).await;
            assert_eq!(st, if round == 0 { 201 } else { 200 }, "{cid}");
        }
    }
    let msgs = e.wait_agent_done(s, 3).await;
    let users: Vec<&str> = msgs
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["body_md"].as_str().unwrap())
        .collect();
    assert_eq!(users, ["message 0", "message 1", "message 2"]);
    assert_eq!(msgs.iter().filter(|m| m["role"] == "agent").count(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_chip_roundtrip() {
    let e = Env::start().await;
    let a = e.admin.make("/personas", common::persona("A")).await["id"]
        .as_i64()
        .unwrap();
    let b = e.admin.make("/personas", common::persona("B")).await["id"]
        .as_i64()
        .unwrap();
    let s = e
        .chat
        .make("/sessions", json!({"name": "s", "persona_id": a}))
        .await["id"]
        .as_i64()
        .unwrap();
    let t = e
        .chat
        .make(&format!("/sessions/{s}/threads"), json!({"name": "thread"}))
        .await["id"]
        .as_i64()
        .unwrap();
    let mut ws = e.ws();
    ws.subscribe("sessions");
    let (st, v) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"persona_id": b}))
        .await;
    assert_eq!((st, v["persona_id"].as_i64()), (200, Some(b)), "{v}");
    ws.until(Duration::from_secs(5), |f| {
        f.data["op"] == "session.updated" && f.data["session"]["id"] == s
    });
    let (_, thread) = e.read.get(&format!("/sessions/{t}")).await;
    assert_eq!(
        thread["effective_persona_id"], b,
        "thread without its own persona inherited it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_removes_messages() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_shell ls").await;
    e.wait_agent_done(s, 2).await;
    let (st, _) = e.chat.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 204);
    assert_eq!(e.read.get(&format!("/sessions/{s}/messages")).await.0, 404);
    let (msgs, row): (i64, (Option<i64>, Option<String>)) = e
        .inspect(|c| {
            let n: i64 = c.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
            let row = c.query_row(
                "SELECT session_id, args_json FROM actions_log WHERE action = 'shell'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok((n, row))
        })
        .await;
    assert_eq!(
        (msgs, row),
        (0, (None, None)),
        "messages are gone, the log remains without session and arguments"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_not_in_client_logs() {
    let e = Env::start_trace().await;
    let secret = "sk-acc-SECRET-77";
    assert_eq!(
        e.admin
            .put("/secrets/provider.anthropic", json!({"value": secret}))
            .await
            .0,
        200
    );
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "#scn:action_send hello").await;
    let (id, _) = e.wait_card(s).await;
    e.decide(id, json!({"decision": "deny"})).await;
    e.wait_agent_done(s, 2).await;
    let log = std::fs::read_to_string(&e.log).unwrap_or_default();
    for (what, v) in [
        ("secret", secret),
        ("admin", &e.admin_token),
        ("chat", &e.chat_token),
        ("agent", &e.agent_token),
        ("module", &e.module_token),
    ] {
        assert!(!log.contains(v), "{what} in the core log");
    }
}
