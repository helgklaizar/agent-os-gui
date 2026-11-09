//! `ConnectionController`: connection state, banners, reconnect delays.

use crate::time::Instant;
use std::time::Duration;

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::types::ConnState;

pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Delay before attempt number `attempt` (from 1): 1, 2, 4, 8, 16, 30, 30… seconds.
pub fn backoff(attempt: u32) -> Duration {
    // 2^5 = 32 s is already above the cap: no further shift is needed (and it cannot overflow)
    let exp = attempt.max(1).saturating_sub(1).min(5);
    Duration::from_secs(1u64 << exp).min(BACKOFF_MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMsg {
    /// The client started its first connection.
    Connecting,
    Connected,
    /// An established connection dropped.
    Dropped,
    /// Another reconnect attempt failed.
    AttemptFailed,
    /// No network or socket.
    Offline,
    /// The core reported `core_unavailable`.
    CoreUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionView {
    pub state: ConnState,
    pub attempt: u32,
    pub retry_at: Option<Instant>,
    pub banner_offline: bool,
    pub banner_core_unavailable: bool,
}

pub struct ConnectionController {
    view: ConnectionView,
}

impl Default for ConnectionController {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionController {
    pub fn new() -> Self {
        Self {
            view: Self::view_for(ConnState::Connecting, 0, None),
        }
    }

    fn view_for(state: ConnState, attempt: u32, retry_at: Option<Instant>) -> ConnectionView {
        ConnectionView {
            state,
            attempt,
            retry_at,
            banner_offline: matches!(state, ConnState::Reconnecting | ConnState::Offline),
            banner_core_unavailable: state == ConnState::CoreUnavailable,
        }
    }

    /// Apply the new state; only an actual state change is published.
    fn set(&mut self, state: ConnState, attempt: u32, retry_at: Option<Instant>) -> Vec<Effect> {
        let changed = state != self.view.state;
        self.view = Self::view_for(state, attempt, retry_at);
        if changed {
            vec![Effect::Publish(BusEvent::ConnectionChanged(state))]
        } else {
            Vec::new()
        }
    }
}

impl Controller for ConnectionController {
    type Msg = ConnectionMsg;
    type View = ConnectionView;

    fn update(&mut self, msg: ConnectionMsg, now: Instant) -> Vec<Effect> {
        let ConnectionView {
            state,
            attempt,
            retry_at,
            ..
        } = self.view;
        match msg {
            ConnectionMsg::Connecting => self.set(ConnState::Connecting, 0, None),
            ConnectionMsg::Connected => self.set(ConnState::Connected, 0, None),
            ConnectionMsg::Dropped => self.set(ConnState::Reconnecting, 1, Some(now + backoff(1))),
            ConnectionMsg::AttemptFailed => match state {
                // a late attempt report while the connection is alive
                ConnState::Connected => Vec::new(),
                // first failure at startup or a repeat in Reconnecting; Offline and CoreUnavailable stay as they are
                ConnState::Connecting
                | ConnState::Reconnecting
                | ConnState::Offline
                | ConnState::CoreUnavailable => {
                    let next = attempt.saturating_add(1);
                    let target = if matches!(state, ConnState::Connecting | ConnState::Reconnecting)
                    {
                        ConnState::Reconnecting
                    } else {
                        state
                    };
                    self.set(target, next, Some(now + backoff(next)))
                }
            },
            ConnectionMsg::Offline => self.set(ConnState::Offline, attempt, retry_at),
            ConnectionMsg::CoreUnavailable => {
                self.set(ConnState::CoreUnavailable, attempt, retry_at)
            }
        }
    }

    fn view(&self) -> ConnectionView {
        self.view
    }
}
