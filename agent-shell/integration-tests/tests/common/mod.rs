//! Shared helpers of the integration tests: the core mock over a real socket and a client against it.
#![allow(dead_code)]

use std::time::Duration;

use async_channel::Receiver;
use core_client::api::Frame;
use core_client::http::{RawResponse, Request};
use core_client::ws::{WsConn, WsIn};
use core_client::{
    ConnState, CoreClient, CoreConfig, CoreEvent, Endpoint, HttpClient, ReconnectPolicy, Token,
    TokenSource,
};
use fake_agentd::{Config, FakeAgentd, Kind, Scope, TokenSpec};

pub const WAIT: Duration = Duration::from_secs(5);

pub async fn start(config: Config) -> FakeAgentd {
    match FakeAgentd::start(config).await {
        Ok(d) => d,
        Err(e) => panic!("fake-agentd did not start: {e}"),
    }
}

/// A mock with a real clock and without a periodic `bar.sys`.
pub async fn quiet() -> FakeAgentd {
    start(Config::temp().no_ticker()).await
}

pub fn unix(d: &FakeAgentd) -> Endpoint {
    Endpoint::Unix(d.socket_path().to_path_buf())
}

pub fn tcp(d: &FakeAgentd) -> Endpoint {
    match d.tcp_addr() {
        Some(a) => Endpoint::Tcp(a.to_string()),
        None => panic!("TCP is not enabled in Config"),
    }
}

pub fn http(d: &FakeAgentd, token: Option<&str>) -> HttpClient {
    HttpClient::new(unix(d), token.map(Token::new)).with_timeout(WAIT)
}

pub fn issue(d: &FakeAgentd, scope: Scope, kind: Kind) -> String {
    match d.issue(TokenSpec::new(scope, kind)) {
        Ok(t) => t.token,
        Err(e) => panic!("token not issued: {e}"),
    }
}

/// A fast reconnect policy for tests; the series and ceiling are checked separately in `core-client`.
pub fn fast_policy() -> ReconnectPolicy {
    ReconnectPolicy {
        base: Duration::from_millis(20),
        max: Duration::from_millis(100),
        jitter: 0.2,
    }
}

pub fn core_config(d: &FakeAgentd, token: &str) -> CoreConfig {
    core_config_with(d, TokenSource::Value(Token::new(token)))
}

pub fn core_config_with(d: &FakeAgentd, source: TokenSource) -> CoreConfig {
    let mut c = CoreConfig::new(unix(d), source);
    c.reconnect = fast_policy();
    c.ping_interval = Duration::from_secs(60);
    c.request_timeout = Duration::from_secs(2);
    c.sync_timeout = Duration::from_millis(500);
    c
}

pub fn client(d: &FakeAgentd) -> CoreClient {
    match CoreClient::start(core_config(d, d.admin_token())) {
        Ok(c) => c,
        Err(e) => panic!("client did not start: {e}"),
    }
}

/// Reads events until the needed one arrives; an error with the tail of what was read if it did not.
pub async fn wait_event<F: Fn(&CoreEvent) -> bool>(
    rx: &Receiver<CoreEvent>,
    what: &str,
    pred: F,
) -> Result<CoreEvent, String> {
    let mut seen = Vec::new();
    let res = tokio::time::timeout(WAIT, async {
        loop {
            match rx.recv().await {
                Ok(ev) if pred(&ev) => return Ok(ev),
                Ok(ev) => seen.push(format!("{ev:?}")),
                Err(_) => return Err("event channel closed".to_string()),
            }
        }
    })
    .await;
    match res {
        Ok(r) => r.map_err(|e| format!("{what}: {e}; seen: {seen:?}")),
        Err(_) => Err(format!("{what}: timed out after {WAIT:?}; seen: {seen:?}")),
    }
}

pub async fn wait_state(rx: &Receiver<CoreEvent>, want: ConnState) -> Result<(), String> {
    wait_event(rx, &format!("state {want:?}"), |e| {
        *e == CoreEvent::State(want)
    })
    .await
    .map(|_| ())
}

/// SHA-256 in hex by an independent implementation (`sha2` in tests), not by the mock code.
pub fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub async fn send(c: &HttpClient, req: Request) -> RawResponse {
    match c.send(req).await {
        Ok(r) => r,
        Err(e) => panic!("request did not reach the mock: {e}"),
    }
}

/// `error.code` and `error.details` from the response body.
pub fn error_of(r: &RawResponse) -> (String, serde_json::Value) {
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or(serde_json::Value::Null);
    (
        v["error"]["code"].as_str().unwrap_or("").to_string(),
        v["error"]["details"].clone(),
    )
}

pub async fn first_ws_message(ws: &mut WsConn) -> Option<WsIn> {
    match tokio::time::timeout(WAIT, ws.next()).await {
        Ok(Some(Ok(m))) => Some(m),
        _ => None,
    }
}

pub fn frame_of(ev: &CoreEvent) -> Option<&Frame> {
    match ev {
        CoreEvent::Frame(f) => Some(f),
        _ => None,
    }
}

pub async fn next_frame(rx: &Receiver<CoreEvent>, topic: &str) -> Result<Frame, String> {
    let ev = wait_event(rx, &format!("frame {topic}"), |e| {
        frame_of(e).is_some_and(|f| f.topic == topic)
    })
    .await?;
    frame_of(&ev)
        .cloned()
        .ok_or_else(|| "not a frame".to_string())
}
