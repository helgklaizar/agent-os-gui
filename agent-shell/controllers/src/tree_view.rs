//! Session tree view (S3-03): a flat list of rows with depth, badges and state; GTK only draws it.

use crate::tree_types::{FeedEvent, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Folder,
    Session,
    Thread,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub node: NodeId,
    pub kind: RowKind,
    /// Nesting level: root is 0.
    pub depth: usize,
    /// Indent in pixels (by depth).
    pub indent: u32,
    /// Display name: a long one is truncated, text is neither escaped nor interpreted (output goes to the view, no markup).
    pub name: String,
    /// Effective persona color (dot); for a folder - its own.
    pub persona_color: Option<String>,
    pub unread: i64,
    /// "3", "99+" or `None`.
    pub badge: Option<String>,
    pub selected: bool,
    pub focused: bool,
    /// `Some` only for folders.
    pub expanded: Option<bool>,
    /// Name for accessibility technologies: title, role and unread count.
    pub a11y_name: String,
}

/// Collapsed column badge: session with unread items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripItem {
    pub session_id: i64,
    /// First letter of the title.
    pub initial: String,
    pub unread: i64,
    pub badge: String,
    pub label: String,
}

/// Collapsed column strip (64 px): the overall indicator on the column icon and badges of sessions with unread items.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Strip {
    pub total_unread: i64,
    /// "5", "99+" or `None` when nothing is unread.
    pub badge: Option<String>,
    pub items: Vec<StripItem>,
    pub a11y_name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TreeView {
    pub rows: Vec<TreeRow>,
    /// Selected session (by id).
    pub selected: Option<i64>,
    /// Row with keyboard focus.
    pub focus: Option<NodeId>,
    /// Neither folders nor sessions.
    pub empty: bool,
    /// "Create a session on the left" when no session is selected and the tree is empty.
    pub empty_hint: Option<String>,
    pub collapsed: bool,
    /// `Some` only when the column is collapsed.
    pub strip: Option<Strip>,
    /// Feed: events remain after the session is deleted (with `session_id = None`).
    pub feed: Vec<FeedEvent>,
    pub new_session: String,
    pub new_folder: String,
}

/// What the tree knows about a session: inherited on read.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub effective_persona_id: Option<i64>,
    pub path: Option<String>,
    /// Whether the parent context is attached (always `false` for a non-thread).
    pub context_from_parent: bool,
    /// "Parent context: isolated" for a thread with `ctx = none`.
    pub context_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
}
