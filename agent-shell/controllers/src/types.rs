//! Common values that travel over the bus and between layers.

use serde::{Deserialize, Serialize};

pub use crate::tree_types::NodeId;

/// Session tools column tab (architecture section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Session chat (the default tab when opened from the tree and from the feed, architecture section 8.4).
    Chat,
    Browser,
    Terminal,
    SessionSettings,
    Modules,
}

/// Settings dialog page (architecture section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    Account,
    About,
    Personas,
    Models,
    Connections,
    Permissions,
    Security,
    Schedule,
    Modules,
    Tunnels,
    Files,
    Indicators,
    /// Settings root: the dialog opens on the first page (the Agent OS menu item "Settings").
    Root,
}

/// Selected theme (what the core stores in `settings.theme` and the cache in `ui.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Auto,
    Dark,
    Light,
}

/// Theme the view actually draws: `Auto` is already resolved from the system one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedTheme {
    Dark,
    Light,
}

/// Core connection state (D39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    Offline,
    CoreUnavailable,
}

/// Column that can be collapsed manually. The chat column does not collapse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    Notifications,
    Sessions,
    Tools,
}

/// Manual column state: `Auto` (`null` in `ui.json`) leaves the decision to auto-collapse.
/// `ui.json` stores `null | "collapsed" | "expanded"` (see `prefs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Manual {
    #[default]
    Auto,
    Collapsed,
    Expanded,
}

/// Column that gets focus on `Super+1…4` (unlike [`Column`], includes the chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusColumn {
    Notifications,
    Sessions,
    Chat,
    Tools,
}

/// Global shortcut action; executed by the owners (architecture, `ShortcutInvoked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    Search,
    NewSession,
    NewFolder,
    NewThread,
    FocusColumn(FocusColumn),
    CloseTopLayer,
}

/// Agent OS menu item (spec section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuItem {
    About,
    User,
    Settings,
    Logout,
}

/// Dialog kind owned by `DialogController`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogKind {
    About,
    ConfirmLogout,
    NewSession,
    NewFolder,
    NewThread,
    MoveSession,
    Rename,
    ConfirmDeleteSession,
    ConfirmDeleteFolder,
}

/// Button at the bottom of the sessions column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeButton {
    NewSession,
    NewFolder,
}

/// Element focus returns to after the dialog closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusTarget {
    MenuItem(MenuItem),
    /// Session tree row.
    Node(NodeId),
    TreeButton(TreeButton),
}

/// What to do when the quota runs out (`settings.balance.fallback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fallback {
    Local,
    Other,
    Wait,
}

impl Fallback {
    /// Value from the core; unknown - `None` (the "Then" row is not shown).
    pub fn parse(text: &str) -> Option<Fallback> {
        match text {
            "local" => Some(Fallback::Local),
            "other" => Some(Fallback::Other),
            "wait" => Some(Fallback::Wait),
            _ => None,
        }
    }
}
