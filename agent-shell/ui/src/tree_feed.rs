//! Bridge from core-client events to `SessionTreeController`: frames of the `sessions` topic, personas, connection.
//! No GTK: works on macOS too, so it is covered by integration tests against `fake-agentd`.

use std::time::Instant;

use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::tree_types::{parse_delta, parse_personas, parse_snapshot};
use controllers::{Controller, Effect};
use core_client::api::FrameKind;
use core_client::{ConnState, CoreEvent};
use serde_json::Value;

/// Topics the tree subscribes to.
pub const TOPICS: [&str; 1] = ["sessions"];

#[derive(Debug, Default, PartialEq)]
pub struct Routed {
    pub effects: Vec<Effect>,
    /// Personas must be re-read (`GET /v1/personas`).
    pub refresh_personas: bool,
}

/// Core client event -> tree. A frame of the wrong shape and an unknown `op` are skipped (api.md section 2).
pub fn route(tree: &mut SessionTreeController, event: &CoreEvent, now: Instant) -> Routed {
    match event {
        CoreEvent::State(ConnState::Connected) => Routed {
            effects: Vec::new(),
            // personas may have changed after every (re)connect
            refresh_personas: true,
        },
        CoreEvent::Frame(f) if f.topic == "sessions" => {
            let msg = match f.kind {
                FrameKind::Snapshot => parse_snapshot(&f.data)
                    .map(|(folders, sessions)| TreeMsg::Snapshot { folders, sessions }),
                FrameKind::Delta => parse_delta(&f.data).map(TreeMsg::Frame),
                FrameKind::Event => None,
            };
            Routed {
                effects: msg.map_or_else(Vec::new, |m| tree.update(m, now)),
                refresh_personas: false,
            }
        }
        _ => Routed::default(),
    }
}

/// Personas from `GET /v1/personas`.
pub fn apply_personas(
    tree: &mut SessionTreeController,
    personas: &[Value],
    now: Instant,
) -> Vec<Effect> {
    let list = parse_personas(&Value::Array(personas.to_vec()));
    tree.update(TreeMsg::Personas(list), now)
}
