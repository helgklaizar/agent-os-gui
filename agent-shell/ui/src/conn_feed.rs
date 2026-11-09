//! Bridge from core-client connection state to `ConnectionController`. No GTK: works on macOS too.

use std::time::Instant;

use controllers::ConnState;
use controllers::Controller;
use controllers::controllers::connection::{ConnectionController, ConnectionMsg, ConnectionView};
use core_client::{ConnState as Wire, CoreEvent};

/// Holds a `ConnectionController` and feeds it core client events.
#[derive(Default)]
pub struct ConnBridge {
    ctl: ConnectionController,
}

impl ConnBridge {
    pub fn new() -> ConnBridge {
        ConnBridge::default()
    }

    /// Current connection view ("No connection" and "Core does not start" banners).
    pub fn view(&self) -> ConnectionView {
        self.ctl.view()
    }

    /// Client event -> controller; `Some(view)` only if the event carries connection state.
    pub fn on_event(&mut self, event: &CoreEvent, now: Instant) -> Option<ConnectionView> {
        let CoreEvent::State(state) = event else {
            return None;
        };
        let msg = match state {
            Wire::Connecting => ConnectionMsg::Connecting,
            Wire::Connected => ConnectionMsg::Connected,
            // an attempt retry counts only inside Reconnecting itself, otherwise it is a disconnect
            Wire::Reconnecting if self.ctl.view().state == ConnState::Reconnecting => {
                ConnectionMsg::AttemptFailed
            }
            Wire::Reconnecting => ConnectionMsg::Dropped,
            Wire::Offline => ConnectionMsg::Offline,
            Wire::CoreUnavailable => ConnectionMsg::CoreUnavailable,
        };
        // no bus publishing needed here: the bar gets the state from `bar_feed`
        self.ctl.update(msg, now);
        Some(self.ctl.view())
    }
}
