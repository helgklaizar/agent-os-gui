//! Mock methods for tests and scenarios: feed events and the subtype registry.

use crate::{DbError, FakeAgentd, events_db, events_ops, events_sim};

impl FakeAgentd {
    /// Adds an event to the feed (insert and an `events` frame). `None` - repeated `dedup_key`, no event.
    pub fn add_event(&self, e: &events_db::NewEvent<'_>) -> Result<Option<i64>, DbError> {
        events_ops::add(&self.shared, e)
    }

    /// Registers a subtype in the `GET /events/subtypes` registry (a repeat of the same pair replaces the entry).
    pub fn register_subtype(&self, kind: &str, sub: &str, label: &str, icon: Option<&str>) {
        self.shared.registry.register(kind, sub, label, icon);
    }

    /// Emulates intercepting D-Bus `org.freedesktop.Notifications`: a `msg/sys` event with `source = agent-sys` (interception is the core's job).
    pub fn dbus_notification(&self, text: &str) -> Result<Option<i64>, DbError> {
        events_sim::dbus(&self.shared, text)
    }
}
