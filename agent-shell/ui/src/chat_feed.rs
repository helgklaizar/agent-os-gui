//! Bridge from core-client events to `ChatController`: `session.<id>` frames, personas, permissions, session deletion.
//! No GTK: works on macOS too, so it is covered by integration tests against `fake-agentd`.

use std::time::Instant;

use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::permissions::PermTable;
use controllers::tree_types::parse_personas;
use controllers::{Controller, Effect};
use core_client::api::FrameKind;
use core_client::{ClientError, ConnState, CoreClient, CoreEvent};
use serde_json::Value;

/// Session topic (`session.<id>`).
pub fn topic(session_id: i64) -> String {
    format!("session.{session_id}")
}

#[derive(Debug, Default, PartialEq)]
pub struct Routed {
    pub effects: Vec<Effect>,
    /// After a (re)connect personas and permissions may have changed.
    pub refresh: bool,
    /// The active session was deleted (`sessions` frame).
    pub deleted: Option<i64>,
}

/// Session number from `session.<id>`.
fn session_of(topic: &str) -> Option<i64> {
    topic.strip_prefix("session.")?.parse().ok()
}

/// Core client event -> chat. Frames of other sessions are dropped by the controller itself.
pub fn route(chat: &mut ChatController, event: &CoreEvent, now: Instant) -> Routed {
    match event {
        CoreEvent::State(ConnState::Connected) => Routed {
            refresh: true,
            ..Routed::default()
        },
        CoreEvent::Frame(f) if f.topic == "sessions" => Routed {
            deleted: (f.data["op"] == "session.deleted")
                .then(|| f.data["session"]["id"].as_i64())
                .flatten(),
            ..Routed::default()
        },
        CoreEvent::Frame(f) => {
            let Some(session_id) = session_of(&f.topic) else {
                return Routed::default();
            };
            let msg = ChatMsg::Frame {
                session_id,
                snapshot: f.kind == FrameKind::Snapshot,
                data: f.data.clone(),
            };
            Routed {
                effects: chat.update(msg, now),
                ..Routed::default()
            }
        }
        CoreEvent::Failed { client_id, .. } => Routed {
            effects: chat.update(
                ChatMsg::SendFailed {
                    client_id: client_id.clone(),
                },
                now,
            ),
            ..Routed::default()
        },
        _ => Routed::default(),
    }
}

/// Resubscribe to a topic (new snapshot); the first error is returned, not lost.
pub fn resubscribe(client: &CoreClient, topic: &str) -> Result<(), ClientError> {
    client
        .unsubscribe(topic)
        .and_then(|()| client.subscribe(topic))
}

pub fn apply_personas(chat: &mut ChatController, list: &[Value], now: Instant) -> Vec<Effect> {
    chat.update(
        ChatMsg::Personas(parse_personas(&Value::Array(list.to_vec()))),
        now,
    )
}

pub fn apply_permissions(chat: &mut ChatController, rows: &Value, now: Instant) -> Vec<Effect> {
    chat.update(ChatMsg::Permissions(PermTable::from_value(rows)), now)
}
