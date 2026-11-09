//! Server side of WS: subscriptions, snapshot, deltas with `seq`, `pong`, close 4401.

use std::collections::{HashMap, HashSet};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::FrameType;
use crate::hub::{Out, known_topic};
use crate::state::Shared;

/// Who connected: `None` - the token was rejected (close with code 4401).
pub type Who = Option<i64>;

fn type_name(k: FrameType) -> &'static str {
    match k {
        FrameType::Snapshot => "snapshot",
        FrameType::Delta => "delta",
        FrameType::Event => "event",
    }
}

struct Conn<S> {
    ws: WebSocketStream<S>,
    seq: HashMap<String, u64>,
    skip: HashSet<String>,
    subs: HashSet<String>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Conn<S> {
    async fn send_json(&mut self, v: Value) -> bool {
        self.ws.send(Message::text(v.to_string())).await.is_ok()
    }

    async fn frame(&mut self, shared: &Shared, topic: &str, kind: FrameType, data: Value) -> bool {
        let n = self.seq.entry(topic.to_string()).or_insert(0);
        *n += 1;
        if kind != FrameType::Snapshot && self.skip.remove(topic) {
            *n += 1;
        }
        let seq = *n;
        let ts = shared.clock.now_unix();
        self.send_json(
            json!({ "topic": topic, "type": type_name(kind), "seq": seq, "ts": ts, "data": data }),
        )
        .await
    }

    async fn on_client(&mut self, shared: &Shared, text: &str) -> bool {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let topic = v
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match v.get("op").and_then(Value::as_str) {
            Some("ping") if shared.hub.pongs() => self.send_json(json!({ "type": "pong" })).await,
            Some("ping") => true,
            Some("subscribe") if known_topic(&topic) => {
                self.subs.insert(topic.clone());
                self.seq.remove(&topic);
                let snap = match topic
                    .strip_prefix("session.")
                    .and_then(|i| i.parse::<i64>().ok())
                {
                    Some(id) => crate::chat_msgs::snapshot(shared, id),
                    None if topic == "events" => crate::events_ops::snapshot(shared),
                    None if topic == "modules" => {
                        items_snapshot(crate::admin::modules_http::all_items(shared))
                    }
                    None if topic == "connections" => {
                        items_snapshot(crate::admin::accounts_http::all_items(shared))
                    }
                    None => shared.hub.snapshot(&topic),
                };
                self.frame(shared, &topic, FrameType::Snapshot, snap).await
            }
            Some("unsubscribe") => {
                self.subs.remove(&topic);
                true
            }
            _ => {
                let err = json!({ "code": "bad_request", "message": "Unknown operation or topic" });
                self.send_json(json!({ "type": "error", "topic": topic, "error": err }))
                    .await
            }
        }
    }
}

/// `{ "items": [...] }` per api.md section 4.4; on a DB read failure an empty list, not a dropped connection.
fn items_snapshot(items: Result<Vec<Value>, crate::reply::ApiErr>) -> Value {
    json!({ "items": items.unwrap_or_default() })
}

async fn close_4401<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
    let frame = CloseFrame {
        code: CloseCode::Library(4401),
        reason: "".into(),
    };
    let _ = ws.send(Message::Close(Some(frame))).await;
    let _ = ws.close(None).await;
}

/// Drives the connection until it drops, the token is revoked or the client closes.
pub async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    shared: &Shared,
    ws: WebSocketStream<S>,
    who: Who,
) {
    let mut ws = ws;
    let Some(token_id) = who else {
        close_4401(&mut ws).await;
        return;
    };
    let mut rx = shared.hub.listen();
    shared.hub.conn_opened();
    let mut c = Conn {
        ws,
        seq: HashMap::new(),
        skip: HashSet::new(),
        subs: HashSet::new(),
    };
    loop {
        tokio::select! {
            msg = c.ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if !c.on_client(shared, t.as_str()).await {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            out = rx.recv() => match out {
                Ok(Out::Frame { topic, kind, data }) => {
                    if c.subs.contains(&topic) && !c.frame(shared, &topic, kind, data).await {
                        break;
                    }
                }
                Ok(Out::Drop) | Err(_) => break,
                Ok(Out::Revoke(id)) if id == token_id => {
                    close_4401(&mut c.ws).await;
                    break;
                }
                Ok(Out::Revoke(_)) => {}
                Ok(Out::SkipSeq(topic)) => {
                    c.skip.insert(topic);
                }
            },
        }
    }
    shared.hub.conn_closed();
}
