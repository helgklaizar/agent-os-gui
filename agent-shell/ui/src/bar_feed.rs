//! Bridge from core-client events to `BarCluster`: `bar.*` frames, connection state, re-reading `balance.fallback`.
//! No GTK: works on macOS too, so it is covered by integration tests against `fake-agentd`.

use std::time::Instant;

use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::{ConnState, Effect, Fallback};
use core_client::api::FrameKind as WireKind;
use core_client::{ConnState as WireConn, CoreEvent};

/// Topics the bar subscribes to (`settings` only for `balance.fallback`).
pub const TOPICS: [&str; 4] = ["bar.sys", "bar.quota", "bar.weather", "settings"];

/// Result of parsing one event.
#[derive(Debug, Default, PartialEq)]
pub struct Routed {
    pub effects: Vec<Effect>,
    /// `balance.fallback` must be re-read from the core.
    pub refresh_fallback: bool,
}

fn conn(state: WireConn) -> ConnState {
    match state {
        WireConn::Connecting => ConnState::Connecting,
        WireConn::Connected => ConnState::Connected,
        WireConn::Reconnecting => ConnState::Reconnecting,
        WireConn::Offline => ConnState::Offline,
        WireConn::CoreUnavailable => ConnState::CoreUnavailable,
    }
}

fn kind(kind: WireKind) -> FrameKind {
    match kind {
        WireKind::Snapshot => FrameKind::Snapshot,
        WireKind::Delta => FrameKind::Delta,
        WireKind::Event => FrameKind::Event,
    }
}

/// Core client event -> bar controllers. `unix` is the current time for "resets in".
pub fn route(cluster: &mut BarCluster, event: &CoreEvent, unix: i64, now: Instant) -> Routed {
    match event {
        CoreEvent::State(state) => Routed {
            effects: cluster.on_connection(conn(*state), now),
            // the setting may have changed after every (re)connect
            refresh_fallback: *state == WireConn::Connected,
        },
        CoreEvent::Frame(f) if f.topic == "settings" => Routed {
            effects: Vec::new(),
            refresh_fallback: f.data.get("page").and_then(|p| p.as_str()) == Some("personas"),
        },
        CoreEvent::Frame(f) => Routed {
            effects: cluster.on_frame(&f.topic, kind(f.kind), &f.data, unix, now),
            refresh_fallback: false,
        },
        _ => Routed::default(),
    }
}

/// `balance.fallback` value from the core (`None` means not read or unrecognised).
pub fn apply_fallback(cluster: &mut BarCluster, value: Option<&str>, now: Instant) -> Vec<Effect> {
    cluster.set_fallback(value.and_then(Fallback::parse), now)
}
