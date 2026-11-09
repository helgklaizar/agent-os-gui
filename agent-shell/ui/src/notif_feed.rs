//! Bridge from core-client events to `NotificationsController`: frames of the `events` and `sessions` topics, subtype registry.
//! No GTK: works on macOS too, so it is covered by integration tests against `fake-agentd`.

use std::time::Instant;

use controllers::controllers::notifications::{NotifMsg, NotificationsController};
use controllers::notif_types::{parse_event, parse_read, parse_snapshot, parse_subtypes};
use controllers::{Controller, Effect, tree_types};
use core_client::api::FrameKind;
use core_client::{ConnState, CoreEvent};
use serde_json::Value;

/// Feed topic; the sessions column subscribes to `sessions` for session names on cards.
pub const TOPICS: [&str; 1] = ["events"];

#[derive(Debug, Default, PartialEq)]
pub struct Routed {
    pub effects: Vec<Effect>,
    /// The subtype registry must be re-read (`GET /v1/events/subtypes`).
    pub refresh_subtypes: bool,
}

/// Core client event -> feed. A frame of the wrong shape and an unknown event `kind` are skipped (api.md section 2).
pub fn route(n: &mut NotificationsController, event: &CoreEvent, now: Instant) -> Routed {
    match event {
        CoreEvent::State(ConnState::Connected) => Routed {
            effects: Vec::new(),
            // the registry may have changed after every (re)connect
            refresh_subtypes: true,
        },
        CoreEvent::Frame(f) if f.topic == "events" => {
            let msg = match f.kind {
                FrameKind::Snapshot => parse_snapshot(&f.data)
                    .map(|(items, counts)| NotifMsg::Snapshot { items, counts }),
                FrameKind::Event => parse_event(&f.data).map(NotifMsg::Created),
                FrameKind::Delta => parse_read(&f.data).map(NotifMsg::Read),
            };
            routed(n, msg, now)
        }
        // session names for cards: the same topic as the tree
        CoreEvent::Frame(f) if f.topic == "sessions" => {
            let msg = match f.kind {
                FrameKind::Snapshot => tree_types::parse_snapshot(&f.data)
                    .map(|(_, sessions)| NotifMsg::Sessions(sessions)),
                FrameKind::Delta => tree_types::parse_delta(&f.data).map(NotifMsg::Tree),
                FrameKind::Event => None,
            };
            routed(n, msg, now)
        }
        _ => Routed::default(),
    }
}

fn routed(n: &mut NotificationsController, msg: Option<NotifMsg>, now: Instant) -> Routed {
    Routed {
        effects: msg.map_or_else(Vec::new, |m| n.update(m, now)),
        refresh_subtypes: false,
    }
}

/// Subtype registry from `GET /v1/events/subtypes`.
pub fn apply_subtypes(
    n: &mut NotificationsController,
    list: &[Value],
    now: Instant,
) -> Vec<Effect> {
    let items = parse_subtypes(&Value::Array(list.to_vec()));
    n.update(NotifMsg::Subtypes(items), now)
}
