//! A1-10: WS hub: subscriptions, `snapshot` before `delta`, `seq`, `ping`/`pong`, close codes.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agentd_api::{Hub, Server, TopicSource};
use agentd_core::{PortFuture, Principal, Scope, TokenKind};
use common::{StaticAuth, config};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{WebSocketStream, client_async};

type Ws = WebSocketStream<UnixStream>;

const T_READ: &str = "agt_ws_read";
const T_OTHER: &str = "agt_ws_other";
const T_ADMIN: &str = "agt_ws_admin";
const T_SHORT: &str = "agt_ws_short";
const T_MODULE: &str = "agt_ws_module";
const T_MODULE_WS: &str = "agt_ws_module_ws";

struct Fixed(Value);

impl TopicSource for Fixed {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> PortFuture<'a, Option<Value>> {
        let v = self.0.clone();
        Box::pin(async move { Some(v) })
    }
}

fn principal(id: i64, scope: Scope, expires_at: Option<i64>) -> Principal {
    Principal {
        token_id: id,
        kind: TokenKind::External,
        scope,
        allowlist: None,
        expires_at,
    }
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

async fn start(dir: &std::path::Path, tweak: impl FnOnce(&mut agentd_api::ServerConfig)) -> Server {
    let mut m = HashMap::new();
    m.insert(T_READ.to_string(), principal(11, Scope::Read, None));
    m.insert(T_OTHER.to_string(), principal(12, Scope::Read, None));
    m.insert(T_ADMIN.to_string(), principal(13, Scope::Admin, None));
    m.insert(
        T_SHORT.to_string(),
        principal(14, Scope::Read, Some(now() + 2)),
    );
    let module = |id, list: &[&str]| Principal {
        token_id: id,
        kind: TokenKind::Module,
        scope: Scope::Read,
        allowlist: Some(list.iter().map(|s| (*s).to_string()).collect()),
        expires_at: None,
    };
    m.insert(T_MODULE.to_string(), module(15, &["GET /sessions"]));
    m.insert(T_MODULE_WS.to_string(), module(16, &["GET /ws"]));
    let mut cfg = config(dir);
    tweak(&mut cfg);
    let s = Server::start(cfg, Arc::new(StaticAuth(m))).await.unwrap();
    let hub = s.hub();
    hub.register_topic("t.a", Scope::Read, Arc::new(Fixed(json!({"n": 1}))));
    hub.register_topic("t.b", Scope::Read, Arc::new(Fixed(json!({"b": true}))));
    hub.register_topic("t.admin", Scope::Admin, Arc::new(Fixed(json!({}))));
    s
}

async fn connect_with(s: &Server, token: &str, extra: &[(&str, &str)]) -> Ws {
    let stream = UnixStream::connect(s.socket_path()).await.unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    for (k, v) in extra {
        let name =
            tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(k.as_bytes()).unwrap();
        req.headers_mut().insert(name, v.parse().unwrap());
    }
    client_async(req, stream).await.expect("WS open").0
}

async fn connect(s: &Server, token: &str) -> Ws {
    connect_with(s, token, &[]).await
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::text(v.to_string())).await.unwrap();
}

/// Next text frame as JSON; everything else (ping/pong) is skipped.
async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("frame arrived in time")
        {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).unwrap(),
            Some(Ok(Message::Close(f))) => panic!("unexpected close: {f:?}"),
            Some(Ok(_)) => {}
            other => panic!("stream broken: {other:?}"),
        }
    }
}

async fn close_code(ws: &mut Ws) -> u16 {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("close arrived in time")
        {
            Some(Ok(Message::Close(Some(f)))) => return u16::from(f.code),
            Some(Ok(Message::Close(None))) | None => panic!("close without a code"),
            Some(Ok(_)) => {}
            Some(Err(e)) => panic!("WS error: {e}"),
        }
    }
}

/// Quiet: no frames in a short time.
async fn expect_quiet(ws: &mut Ws) {
    let got = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(Ok(Message::Text(t))) = ws.next().await {
                return t.to_string();
            }
        }
    })
    .await;
    assert!(got.is_err(), "extra frame: {got:?}");
}

#[tokio::test]
async fn ws_snapshot_before_delta() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let hub: Hub = s.hub();
    hub.publish("t.a", json!({"lost": true})); // before subscribing: not delivered
    for round in 0..15 {
        let mut ws = connect(&s, T_READ).await;
        // Publishing runs continuously while the client subscribes: snapshot still arrives first.
        let h = hub.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let publisher = tokio::spawn(async move {
            let mut i = 0;
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                h.publish("t.a", json!({"i": i}));
                i += 1;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        send(&mut ws, json!({"op": "subscribe", "topic": "t.a"})).await;
        let first = next_json(&mut ws).await;
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        publisher.await.unwrap();
        assert_eq!(first["type"], "snapshot", "round {round}: {first}");
        assert_eq!(first["topic"], "t.a");
        assert_eq!(first["seq"], 1);
        assert_eq!(first["data"], json!({"n": 1}));
        assert!(first["ts"].as_i64().is_some_and(|t| t > 1_700_000_000));
        ws.close(None).await.ok();
    }
    s.shutdown().await;
}

#[tokio::test]
async fn ws_seq_monotonic() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let hub = s.hub();
    let mut a = connect(&s, T_READ).await;
    let mut b = connect(&s, T_OTHER).await;
    for ws in [&mut a, &mut b] {
        send(ws, json!({"op": "subscribe", "topic": "t.a"})).await;
        assert_eq!(next_json(ws).await["seq"], 1);
    }
    send(&mut a, json!({"op": "subscribe", "topic": "t.b"})).await;
    assert_eq!(
        next_json(&mut a).await["seq"],
        1,
        "each topic has its own counter"
    );
    for i in 0..50 {
        hub.publish("t.a", json!({"i": i}));
    }
    hub.publish("t.b", json!({"x": 1}));
    for ws in [&mut a, &mut b] {
        for want in 2..52 {
            let m = next_json(ws).await;
            assert_eq!(
                (m["type"].as_str(), m["seq"].as_u64()),
                (Some("delta"), Some(want)),
                "{m}"
            );
        }
    }
    let m = next_json(&mut a).await;
    assert_eq!(
        (m["topic"].clone(), m["seq"].clone()),
        (json!("t.b"), json!(2))
    );
    expect_quiet(&mut b).await; // b is not subscribed to t.b
    // Events (`event`) use the same numbering.
    hub.publish_event("t.a", json!({"k": 1}));
    let m = next_json(&mut a).await;
    assert_eq!(
        (m["type"].clone(), m["seq"].clone()),
        (json!("event"), json!(52))
    );
    s.shutdown().await;
}

#[tokio::test]
async fn ws_unsubscribe_and_resubscribe() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let hub = s.hub();
    let mut ws = connect(&s, T_READ).await;
    send(&mut ws, json!({"op": "subscribe", "topic": "t.a"})).await;
    next_json(&mut ws).await;
    hub.publish("t.a", json!(1));
    assert_eq!(next_json(&mut ws).await["seq"], 2);
    send(&mut ws, json!({"op": "unsubscribe", "topic": "t.a"})).await;
    send(&mut ws, json!({"op": "ping"})).await;
    assert_eq!(next_json(&mut ws).await["type"], "pong");
    hub.publish("t.a", json!(2));
    expect_quiet(&mut ws).await;
    send(&mut ws, json!({"op": "subscribe", "topic": "t.a"})).await;
    let m = next_json(&mut ws).await;
    assert_eq!(
        (m["type"].clone(), m["seq"].clone()),
        (json!("snapshot"), json!(1)),
        "new subscription: new snapshot and counter"
    );
    s.shutdown().await;
}

#[tokio::test]
async fn ws_ping_pong() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |c| {
        c.ws_ping_interval = Duration::from_millis(200)
    })
    .await;
    let mut ws = connect(&s, T_READ).await;
    send(&mut ws, json!({"op": "ping"})).await;
    assert_eq!(next_json(&mut ws).await, json!({"type": "pong"}));
    // The core sends ping itself if the client is silent longer than the interval.
    let got = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(Message::Ping(_))) = ws.next().await {
                return;
            }
        }
    })
    .await;
    assert!(got.is_ok(), "core did not send ping after silence");
    s.shutdown().await;
}

#[tokio::test]
async fn ws_revoked_token_closes_4401() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let mut victim = connect(&s, T_READ).await;
    let mut other = connect(&s, T_OTHER).await;
    for ws in [&mut victim, &mut other] {
        send(ws, json!({"op": "subscribe", "topic": "t.a"})).await;
        next_json(ws).await;
    }
    s.hub().revoke_token(11);
    assert_eq!(close_code(&mut victim).await, 4401);
    s.hub().publish("t.a", json!(1));
    assert_eq!(
        next_json(&mut other).await["seq"],
        2,
        "foreign token unaffected"
    );
    s.shutdown().await;
}

#[tokio::test]
async fn ws_expired_token_closes_4401_while_connected() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let mut ws = connect(&s, T_SHORT).await;
    assert_eq!(
        close_code(&mut ws).await,
        4401,
        "token expired while the connection was open"
    );
    s.shutdown().await;
}

#[tokio::test]
async fn ws_scope_below_read_4403() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let mut ws = connect(&s, T_READ).await;
    send(&mut ws, json!({"op": "subscribe", "topic": "t.admin"})).await;
    assert_eq!(close_code(&mut ws).await, 4403);
    let mut ok = connect(&s, T_ADMIN).await;
    send(&mut ok, json!({"op": "subscribe", "topic": "t.admin"})).await;
    assert_eq!(next_json(&mut ok).await["type"], "snapshot");
    s.shutdown().await;
}

#[tokio::test]
async fn ws_bad_ops_get_error_frames_and_connection_survives() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let mut ws = connect(&s, T_READ).await;
    for bad in [
        json!({"op": "subscribe", "topic": "no.such"}),
        json!({"op": "subscribe"}),
        json!({"op": "dance"}),
        json!([1, 2]),
    ] {
        send(&mut ws, bad.clone()).await;
        let m = next_json(&mut ws).await;
        assert_eq!(m["type"], "error", "{bad}: {m}");
        assert!(m["data"]["code"].is_string());
    }
    ws.send(Message::text("{not json")).await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "error");
    send(&mut ws, json!({"op": "ping"})).await;
    assert_eq!(next_json(&mut ws).await["type"], "pong", "connection alive");
    s.shutdown().await;
}

#[tokio::test]
async fn ws_graceful_restart_1001() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let mut ws = connect(&s, T_READ).await;
    send(&mut ws, json!({"op": "subscribe", "topic": "t.a"})).await;
    next_json(&mut ws).await;
    let closer = tokio::spawn(async move { s.shutdown().await });
    assert_eq!(close_code(&mut ws).await, 1001);
    closer.await.unwrap();
}

#[tokio::test]
async fn ws_unsupported_api_major_4426() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    for bad in ["2", "0", "x"] {
        let mut ws = connect_with(&s, T_READ, &[("x-api-major", bad)]).await;
        assert_eq!(close_code(&mut ws).await, 4426, "X-Api-Major: {bad}");
    }
    let mut ws = connect_with(&s, T_READ, &[("x-api-major", "1")]).await;
    send(&mut ws, json!({"op": "ping"})).await;
    assert_eq!(next_json(&mut ws).await["type"], "pong");
    s.shutdown().await;
}

#[tokio::test]
async fn ws_token_with_allowlist_needs_ws_entry() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(dir.path(), |_| {}).await;
    let stream = UnixStream::connect(s.socket_path()).await.unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {T_MODULE}").parse().unwrap(),
    );
    match client_async(req, stream).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status().as_u16(), 403),
        other => panic!(
            "expected handshake rejection 403, got {:?}",
            other.map(|_| ())
        ),
    }
    let mut ok = connect(&s, T_MODULE_WS).await;
    send(&mut ok, json!({"op": "ping"})).await;
    assert_eq!(next_json(&mut ok).await["type"], "pong");
    s.shutdown().await;
}
