//! WS session (docs/reference/api.md 4, A1-10): subscriptions, `snapshot` before `delta`, per-topic `seq` within a connection,
//! `ping`/`pong`, token revocation, token expiry, close codes.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agentd_core::{Principal, Scope};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use crate::hub::Bus;
use crate::state::ApiState;

pub(crate) const CLOSE_GOING_AWAY: u16 = 1001;
pub(crate) const CLOSE_UNAUTHORIZED: u16 = 4401;
pub(crate) const CLOSE_FORBIDDEN: u16 = 4403;
pub(crate) const CLOSE_VERSION: u16 = 4426;

/// Limit on an incoming client frame: WS operations are short.
pub(crate) const MAX_CLIENT_MESSAGE: usize = 64 * 1024;
const MAX_SUBSCRIPTIONS: usize = 64;

pub(crate) async fn close_with(mut socket: WebSocket, code: u16) {
    let frame = CloseFrame {
        code,
        reason: "".into(),
    };
    if socket.send(Message::Close(Some(frame))).await.is_err() {
        tracing::debug!(
            scope = "api",
            event = "ws_close_failed",
            code,
            "client left before close"
        );
    }
}

/// The active connection counter is decremented on any exit from `run`.
struct ConnGuard(Arc<ApiState>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.metrics.ws_connections.fetch_sub(1, Ordering::SeqCst);
    }
}

enum Flow {
    Continue,
    Close(u16),
    Gone,
}

struct Session {
    state: Arc<ApiState>,
    principal: Principal,
    /// Topic -> last sent `seq`.
    subs: HashMap<String, u64>,
}

fn frame(topic: &str, kind: &str, seq: u64, ts: i64, data: &Value) -> String {
    json!({ "topic": topic, "type": kind, "seq": seq, "ts": ts, "data": data }).to_string()
}

fn error_frame(code: &str, topic: Option<&str>) -> String {
    json!({ "type": "error", "topic": topic, "data": { "code": code } }).to_string()
}

async fn send(socket: &mut WebSocket, text: String) -> bool {
    socket.send(Message::text(text)).await.is_ok()
}

impl Drop for Session {
    fn drop(&mut self) {
        for topic in self.subs.keys() {
            self.state.hub.unwatch(topic, self.principal.token_id);
        }
    }
}

impl Session {
    fn drop_sub(&mut self, topic: &str) {
        if self.subs.remove(topic).is_some() {
            self.state.hub.unwatch(topic, self.principal.token_id);
        }
    }

    /// Subscription: topic and scope check, then `snapshot` with `seq = 1`. Bus messages that arrived while the
    /// snapshot was being read sit in the connection queue and go out after it.
    async fn subscribe(&mut self, socket: &mut WebSocket, topic: &str) -> Flow {
        let Some(entry) = self.state.hub.lookup(topic) else {
            return reply(socket, error_frame("unknown_topic", Some(topic))).await;
        };
        if !self
            .principal
            .scope
            .satisfies(entry.min_scope.max(Scope::Read))
        {
            tracing::warn!(
                scope = "api",
                event = "ws_topic_forbidden",
                token_id = self.principal.token_id,
                "subscription above the token scope"
            );
            return Flow::Close(CLOSE_FORBIDDEN);
        }
        if !self.subs.contains_key(topic) && self.subs.len() >= MAX_SUBSCRIPTIONS {
            return reply(socket, error_frame("too_many_subscriptions", Some(topic))).await;
        }
        self.snapshot(socket, topic, &entry.source).await
    }

    async fn snapshot(
        &mut self,
        socket: &mut WebSocket,
        topic: &str,
        source: &Arc<dyn crate::hub::TopicSource>,
    ) -> Flow {
        let Some(data) = source.snapshot(topic).await else {
            self.drop_sub(topic);
            return reply(socket, error_frame("core_down", Some(topic))).await;
        };
        if self.subs.insert(topic.to_string(), 1).is_none() {
            self.state.hub.watch(topic, self.principal.token_id);
        }
        reply(
            socket,
            frame(topic, "snapshot", 1, self.state.hub.now(), &data),
        )
        .await
    }

    async fn on_text(&mut self, socket: &mut WebSocket, text: &str) -> Flow {
        let Ok(Value::Object(op)) = serde_json::from_str::<Value>(text) else {
            return reply(socket, error_frame("bad_request", None)).await;
        };
        let topic = op.get("topic").and_then(Value::as_str);
        match (op.get("op").and_then(Value::as_str), topic) {
            (Some("ping"), _) => reply(socket, json!({ "type": "pong" }).to_string()).await,
            (Some("subscribe"), Some(t)) => self.subscribe(socket, t).await,
            (Some("unsubscribe"), Some(t)) => {
                self.drop_sub(t);
                Flow::Continue
            }
            _ => reply(socket, error_frame("bad_request", topic)).await,
        }
    }

    async fn on_bus(
        &mut self,
        socket: &mut WebSocket,
        ev: Result<Bus, broadcast::error::RecvError>,
    ) -> Flow {
        match ev {
            Ok(Bus::Message {
                topic,
                kind,
                data,
                ts,
            }) => match self.subs.get_mut(&*topic) {
                Some(seq) => {
                    *seq += 1;
                    reply(socket, frame(&topic, kind.as_str(), *seq, ts, &data)).await
                }
                None => Flow::Continue,
            },
            Ok(Bus::Revoked(id)) if id == self.principal.token_id => {
                Flow::Close(CLOSE_UNAUTHORIZED)
            }
            Ok(Bus::Revoked(_)) => Flow::Continue,
            Ok(Bus::Logout) => Flow::Close(CLOSE_GOING_AWAY),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // The connection did not keep up reading: the gap cannot be masked, so send a fresh snapshot and `seq` from 1.
                tracing::warn!(
                    scope = "api",
                    event = "ws_lagged",
                    skipped = n,
                    "subscriber lagged, resending snapshots"
                );
                let topics: Vec<String> = self.subs.keys().cloned().collect();
                for t in topics {
                    if let Some(entry) = self.state.hub.lookup(&t)
                        && let Flow::Close(c) = self.snapshot(socket, &t, &entry.source).await
                    {
                        return Flow::Close(c);
                    }
                }
                Flow::Continue
            }
            Err(broadcast::error::RecvError::Closed) => Flow::Gone,
        }
    }
}

async fn reply(socket: &mut WebSocket, text: String) -> Flow {
    if send(socket, text).await {
        Flow::Continue
    } else {
        Flow::Gone
    }
}

/// Waits for the close signal; the `watch::Ref` guard does not live across the caller's `await`.
async fn signalled(rx: &mut watch::Receiver<bool>) {
    drop(rx.wait_for(|v| *v).await);
}

/// Sleeps until the token stops being valid; without an expiry it never wakes.
async fn until_expiry(state: &ApiState, expires_at: Option<i64>) {
    let Some(exp) = expires_at else {
        return std::future::pending().await;
    };
    loop {
        let left = exp.saturating_sub(state.clock.now_unix());
        if left <= 0 {
            return;
        }
        tokio::time::sleep(Duration::from_secs(
            u64::try_from(left).unwrap_or(1).min(60),
        ))
        .await;
    }
}

pub(crate) async fn run(
    mut socket: WebSocket,
    state: Arc<ApiState>,
    principal: Principal,
    mut closed: watch::Receiver<bool>,
) {
    state.metrics.ws_connections.fetch_add(1, Ordering::SeqCst);
    state.metrics.ws_total.fetch_add(1, Ordering::SeqCst);
    let _guard = ConnGuard(state.clone());
    let mut bus = state.hub.bus();
    let ping_every = state.ping_interval;
    let mut idle = Instant::now() + ping_every;
    let expires_at = principal.expires_at;
    let mut session = Session {
        state: state.clone(),
        principal,
        subs: HashMap::new(),
    };
    loop {
        let flow = tokio::select! {
            msg = socket.recv() => match msg {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(Message::Text(t))) => {
                    idle = Instant::now() + ping_every;
                    session.on_text(&mut socket, t.as_str()).await
                }
                Some(Ok(_)) => { idle = Instant::now() + ping_every; Flow::Continue }
            },
            ev = bus.recv() => session.on_bus(&mut socket, ev).await,
            () = signalled(&mut closed) => Flow::Close(CLOSE_GOING_AWAY),
            () = until_expiry(&state, expires_at) => Flow::Close(CLOSE_UNAUTHORIZED),
            () = tokio::time::sleep_until(idle) => {
                idle = Instant::now() + ping_every;
                if socket.send(Message::Ping(Vec::new().into())).await.is_ok() { Flow::Continue } else { Flow::Gone }
            }
        };
        match flow {
            Flow::Continue => {}
            Flow::Gone => return,
            Flow::Close(code) => {
                close_with(socket, code).await;
                return;
            }
        }
    }
}
