//! Mock WS frames against `api.md` section 4: envelope, first subscription frame, topics from the section 4.3 table, `ping` and error.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::mock;
use crate::rawhttp::{Req, send};

type Ws = tokio_tungstenite::WebSocketStream<UnixStream>;

async fn connect(sock: &std::path::Path, token: &str) -> Ws {
    let stream = UnixStream::connect(sock).await.expect("socket");
    let mut req = "ws://localhost/v1/ws"
        .into_client_request()
        .expect("request");
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().expect("header"),
    );
    match tokio_tungstenite::client_async(req, stream).await {
        Ok((ws, _)) => ws,
        Err(e) => panic!("WS did not open: {e}"),
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let got = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
        match got {
            Ok(Some(Ok(Message::Text(t)))) => {
                return serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
            }
            Ok(Some(Ok(_))) => {}
            other => panic!("no frame: {other:?}"),
        }
    }
}

/// Topics from the `api.md` section 4.3 table (first column, in backticks).
fn documented_topics() -> Vec<String> {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/api.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let section = text
        .split("### 4.3 Topics")
        .nth(1)
        .and_then(|s| s.split("### 4.4").next())
        .unwrap_or("");
    section
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .map(str::to_string)
        .collect()
}

/// Fields that `api.md` section 4.4 promises in a topic `snapshot`.
fn snapshot_keys(topic: &str) -> &'static [&'static str] {
    match topic {
        "bar.sys" => &["cpu", "gpu", "ram", "disk", "temp"],
        "bar.quota" | "bar.indicators" | "modules" | "connections" => &["items"],
        "events" => &["items", "counts"],
        "sessions" => &["folders", "sessions"],
        t if t.starts_with("session.") => &["session", "messages", "open_requests", "tasks"],
        _ => &[],
    }
}

#[test]
fn contract_ws_documented_topics_are_parsed() {
    let topics = documented_topics();
    assert!(
        topics.len() >= 10,
        "api.md section 4.3 must have at least 10 topics: {topics:?}"
    );
    assert!(topics.contains(&"session.<id>".to_string()), "{topics:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_ws_first_frame_is_snapshot_for_every_documented_topic() {
    let d = mock().await;
    let sock = d.socket_path();
    let admin = d.admin_token().to_string();
    let sessions = send(sock, Req::new("GET", "/v1/sessions").token(&admin)).await;
    let sid = sessions.json()[0]["id"].as_i64().unwrap_or(-1);
    assert!(sid > 0, "the mock has a session: {}", sessions.json());
    let mut ws = connect(sock, &admin).await;
    for doc in documented_topics() {
        let topic = doc.replace("<id>", &sid.to_string());
        ws.send(Message::text(
            json!({"op": "subscribe", "topic": topic}).to_string(),
        ))
        .await
        .expect("subscription");
        let f = next_json(&mut ws).await;
        assert_eq!(f["topic"], topic.as_str(), "{topic}: frame topic");
        assert_eq!(f["type"], "snapshot", "{topic}: snapshot comes first: {f}");
        assert_eq!(f["seq"], 1, "{topic}: seq starts at one");
        assert!(
            f["ts"].as_i64().is_some_and(|t| t > 1_700_000_000),
            "{topic}: ts is unix seconds: {f}"
        );
        let obj = f["data"].as_object();
        assert!(obj.is_some(), "{topic}: data is an object: {f}");
        let keys: Vec<&str> = f
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys())
            .map(String::as_str)
            .collect();
        for k in ["topic", "type", "seq", "ts", "data"] {
            assert!(keys.contains(&k), "{topic}: missing envelope field {k}");
        }
        for k in snapshot_keys(&doc) {
            assert!(
                f["data"].get(k).is_some(),
                "{topic}: snapshot lacks field {k} (api.md section 4.4): {}",
                f["data"]
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_ws_ping_pong_and_unknown_op() {
    let d = mock().await;
    let mut ws = connect(d.socket_path(), d.admin_token()).await;
    ws.send(Message::text(json!({"op": "ping"}).to_string()))
        .await
        .expect("ping");
    assert_eq!(next_json(&mut ws).await, json!({"type": "pong"}));
    ws.send(Message::text(
        json!({"op": "subscribe", "topic": "no.such.topic"}).to_string(),
    ))
    .await
    .expect("subscription");
    let e = next_json(&mut ws).await;
    assert_eq!(e["type"], "error", "unknown topic: {e}");
    assert_eq!(e["error"]["code"], "bad_request");
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_ws_seq_grows_by_one_and_resubscribe_restarts() {
    let d = mock().await;
    let mut ws = connect(d.socket_path(), d.admin_token()).await;
    let sub = || Message::text(json!({"op": "subscribe", "topic": "sessions"}).to_string());
    ws.send(sub()).await.expect("repeat");
    assert_eq!(next_json(&mut ws).await["seq"], 1);
    d.emit(
        "sessions",
        fake_agentd::FrameType::Delta,
        json!({"op": "unread", "session_id": 1, "unread": 1}),
    );
    let delta = next_json(&mut ws).await;
    assert_eq!(
        (delta["type"].clone(), delta["seq"].clone()),
        (json!("delta"), json!(2))
    );
    ws.send(sub()).await.expect("subscription");
    let again = next_json(&mut ws).await;
    assert_eq!(
        (again["type"].clone(), again["seq"].clone()),
        (json!("snapshot"), json!(1)),
        "a repeat subscription restarts the count and sends a snapshot"
    );
}
