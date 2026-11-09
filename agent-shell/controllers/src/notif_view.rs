//! Notifications column view: a plain structure without GTK. All text is passed as is; the view escapes it
//! (`set_use_markup(false)`), not the controller.

use crate::notif_types::EventKind;

/// What is selected in the filter panel: `All → kind → subtype`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    All,
    Kind(EventKind),
    Sub(EventKind, String),
}

/// Card colour stripe: AI is the accent, task is yellow, message is green.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Ai,
    Task,
    Msg,
}

/// Card button: an event with a session gets "Open chat", without a session "OK".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardAction {
    #[default]
    OpenChat,
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventCard {
    pub id: i64,
    pub kind: String,
    pub sub: String,
    /// Subtype label from the registry, or `sub` itself if absent.
    pub sub_label: String,
    pub icon: Option<String>,
    pub level: String,
    /// `HH:MM` in the user's timezone.
    pub time: String,
    pub session_id: Option<i64>,
    /// Session name; for a thread, "parent › thread".
    pub session_label: Option<String>,
    pub text: String,
    pub unread: bool,
    /// "New" label on an unread item.
    pub new_label: Option<String>,
    pub accent: Accent,
    pub action: CardAction,
    pub action_label: String,
}

/// Unread badge: empty at zero, "99+" from a hundred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Badge {
    pub count: i64,
    pub text: Option<String>,
    /// "Label: N unread" for the screen reader.
    pub a11y_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubChip {
    pub sub: String,
    pub label: String,
    pub icon: Option<String>,
    pub badge: Badge,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindChip {
    pub kind: EventKind,
    pub label: String,
    pub icon: &'static str,
    pub badge: Badge,
    pub active: bool,
    /// Kind subfilters from the subtype registry (visible when the kind is selected).
    pub subs: Vec<SubChip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AllChip {
    pub label: String,
    pub badge: Badge,
    pub active: bool,
}

/// Kind icon on the collapsed strip (56 px).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripItem {
    pub kind: EventKind,
    pub icon: &'static str,
    pub label: String,
    pub badge: Badge,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifView {
    /// Window title: `(N) Agent OS`, or `Agent OS` when nothing is unread.
    pub title: String,
    pub total_unread: i64,
    pub selection: Selection,
    pub all: AllChip,
    pub kinds: Vec<KindChip>,
    pub cards: Vec<EventCard>,
    /// "Nothing here yet…" or "Nothing matches this filter" when there are no cards.
    pub empty: Option<String>,
    pub collapsed: bool,
    /// Vertical strip of kind icons; empty when the column is expanded.
    pub strip: Vec<StripItem>,
}
