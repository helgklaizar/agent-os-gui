//! Notification feed types as the controllers see them (docs/reference/api.md): events, counters, subtype registry, `events` frames.
//! Unknown fields are ignored; missing ones take default values.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::types::Tab;

/// Event kind: three fixed schema values (`kind IN ('ai','task','msg')`); subtypes are core data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    Ai,
    Task,
    Msg,
}

impl EventKind {
    pub const ALL: [EventKind; 3] = [EventKind::Ai, EventKind::Task, EventKind::Msg];

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Ai => "ai",
            EventKind::Task => "task",
            EventKind::Msg => "msg",
        }
    }

    pub fn parse(text: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

/// `Event` from docs/reference/api.md (`dedup_key` is not exposed to the client).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct EventItem {
    pub id: i64,
    pub kind: String,
    pub sub: String,
    pub level: String,
    pub text: String,
    pub session_id: Option<i64>,
    /// Computed by the core, `chat` by default; an empty value is read as `chat`.
    pub tab: String,
    pub source: Option<String>,
    pub unread: bool,
    pub ts: i64,
}

/// `EventCounts`: unread in total, per kind and per subtype (key `kind.sub`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct EventCounts {
    pub total: i64,
    pub by_kind: BTreeMap<String, i64>,
    pub by_sub: BTreeMap<String, i64>,
}

fn enabled_by_default() -> bool {
    true
}

/// A `GET /events/subtypes` registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubtypeItem {
    pub kind: String,
    pub sub: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

/// The "read" filter (`POST /events/read`): empty means all unread.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventsFilter {
    pub kind: Option<EventKind>,
    pub sub: Option<String>,
    pub session_id: Option<i64>,
}

/// Tools column tab by `Event.tab`; an empty or unknown value means chat (docs/reference/api.md).
pub fn tab_of(tab: &str) -> Tab {
    match tab {
        "browser" => Tab::Browser,
        "terminal" => Tab::Terminal,
        "settings" => Tab::SessionSettings,
        _ => Tab::Chat,
    }
}

/// An event from an `event` frame body or an `items` element; an unknown `kind` gives `None` (the frame is skipped, docs/reference/api.md §2).
pub fn parse_event(data: &Value) -> Option<EventItem> {
    let item: EventItem = serde_json::from_value(data.clone()).ok()?;
    data.get("id")?;
    EventKind::parse(&item.kind)?;
    Some(item)
}

/// An `events` snapshot: `{ items, counts }`; a malformed frame gives `None`, malformed elements are skipped.
pub fn parse_snapshot(data: &Value) -> Option<(Vec<EventItem>, EventCounts)> {
    let items = data
        .get("items")?
        .as_array()?
        .iter()
        .filter_map(parse_event)
        .collect();
    let counts = match data.get("counts") {
        Some(c) => serde_json::from_value(c.clone()).ok()?,
        None => EventCounts::default(),
    };
    Some((items, counts))
}

/// An `events` delta: `{ read: [id…] }`.
pub fn parse_read(data: &Value) -> Option<Vec<i64>> {
    data.get("read")?
        .as_array()?
        .iter()
        .map(Value::as_i64)
        .collect()
}

/// `GET /events/subtypes` response: a malformed entry is skipped.
pub fn parse_subtypes(data: &Value) -> Vec<SubtypeItem> {
    data.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}
