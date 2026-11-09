//! WS frames (api.md section 4): parsing of server messages and client operations.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Snapshot,
    Delta,
    Event,
}

/// Subscription frame (api.md section 4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub topic: String,
    pub kind: FrameKind,
    pub seq: u64,
    pub ts: i64,
    pub data: Value,
}

/// A server message over WS.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Pong,
    Frame(Frame),
}

/// Parsing of a server text frame; unknown `type` and garbage give `None` (ignored, api.md section 2).
pub fn parse_server_message(text: &str) -> Option<ServerMessage> {
    let v: Value = serde_json::from_str(text).ok()?;
    let kind = match v.get("type")?.as_str()? {
        "pong" => return Some(ServerMessage::Pong),
        "snapshot" => FrameKind::Snapshot,
        "delta" => FrameKind::Delta,
        "event" => FrameKind::Event,
        _ => return None,
    };
    Some(ServerMessage::Frame(Frame {
        topic: v.get("topic")?.as_str()?.to_string(),
        kind,
        seq: v.get("seq")?.as_u64()?,
        ts: v.get("ts").and_then(Value::as_i64).unwrap_or(0),
        data: v.get("data").cloned().unwrap_or(Value::Null),
    }))
}

/// Client messages (api.md section 4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientOp {
    Subscribe(String),
    Unsubscribe(String),
    Ping,
}

impl ClientOp {
    pub fn to_json(&self) -> String {
        match self {
            ClientOp::Subscribe(t) => json!({ "op": "subscribe", "topic": t }),
            ClientOp::Unsubscribe(t) => json!({ "op": "unsubscribe", "topic": t }),
            ClientOp::Ping => json!({ "op": "ping" }),
        }
        .to_string()
    }
}
