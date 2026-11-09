//! Simulation-mode events: an agent reply as a session event, D-Bus emulation.

use crate::chat_msgs;
use crate::events_db::NewEvent;
use crate::events_ops;
use crate::state::Shared;
use crate::tree_session_ops;

/// Label of the reply event: the start of the text without line breaks, at most 80 characters.
fn headline(body: &str) -> String {
    let one: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(80).collect()
}

pub fn agent_finished(shared: &Shared, session: i64, message_id: i64, status: &str) {
    if status == "done" {
        agent_replied(shared, session, message_id);
    }
}

/// A finished agent reply: an `ai/agent` session event and +1 to its unread (in the real core a message arrival does this).
/// A stopped or interrupted reply does not become an event.
fn agent_replied(shared: &Shared, session: i64, message_id: i64) {
    let message = shared
        .db
        .reader()
        .ok()
        .and_then(|c| chat_msgs::get(&c, message_id).ok().flatten())
        .unwrap_or_default();
    let text = headline(message["body_md"].as_str().unwrap_or(""));
    let text = if text.is_empty() {
        "Agent reply".to_string()
    } else {
        text
    };
    let event = NewEvent {
        kind: "ai",
        sub: "agent",
        level: Some("ok"),
        text: &text,
        session_id: Some(session),
        source: Some("agentd"),
        ts: shared.clock.now_unix(),
        ..NewEvent::default()
    };
    if let Err(e) = events_ops::add(shared, &event) {
        tracing::error!(error = %e, "agent reply event not written");
    }
    if let Err(e) = tree_session_ops::add_unread(shared, session, 1) {
        tracing::error!(error = ?e, "session unread not updated");
    }
}

/// Session bus notification: `msg/sys` from `agent-sys`.
pub fn dbus(shared: &Shared, text: &str) -> Result<Option<i64>, crate::DbError> {
    events_ops::add(
        shared,
        &NewEvent {
            kind: "msg",
            sub: "sys",
            text,
            source: Some("agent-sys"),
            ts: shared.clock.now_unix(),
            ..NewEvent::default()
        },
    )
}
