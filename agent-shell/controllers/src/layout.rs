//! Column layout: a pure function of the window width and manual states.

use crate::prefs::ColumnsPrefs;
use crate::types::Manual;

pub const NOTIFICATIONS_EXPANDED: u32 = 300;
pub const SESSIONS_EXPANDED: u32 = 236;
pub const TOOLS_EXPANDED: u32 = 560;
pub const NOTIFICATIONS_COLLAPSED: u32 = 56;
pub const SESSIONS_COLLAPSED: u32 = 64;
pub const TOOLS_COLLAPSED: u32 = 56;
pub const CHAT_MIN: u32 = 480;
/// Minimum window size (width, height).
pub const MIN_WINDOW: (u32, u32) = (1024, 640);
/// Below this window width the tools column auto-collapses.
pub const TOOLS_AUTO_COLLAPSE_BELOW: u32 = 1576;
/// Below this window width the notifications column auto-collapses.
pub const NOTIFICATIONS_AUTO_COLLAPSE_BELOW: u32 = 1072;

/// Layout result: whether a column is collapsed and the widths in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutState {
    pub notifications_collapsed: bool,
    pub sessions_collapsed: bool,
    pub tools_collapsed: bool,
    pub notifications_w: u32,
    pub sessions_w: u32,
    pub tools_w: u32,
    pub chat_w: u32,
}

/// Layout for window width `window_w` and manual column states.
///
/// A manual state always wins over an automatic one. If the chat is left with less than [`CHAT_MIN`], first
/// the columns without a manual state auto-collapse (notifications, then tools); if that is not enough,
/// the chat shrinks and the manual choice is not overridden. The sessions column never auto-collapses.
pub fn resolve(window_w: u32, manual: &ColumnsPrefs) -> LayoutState {
    let mut notifications = collapsed_by(
        manual.notifications.manual,
        window_w < NOTIFICATIONS_AUTO_COLLAPSE_BELOW,
    );
    let sessions = collapsed_by(manual.sessions.manual, false);
    let mut tools = collapsed_by(manual.tools.manual, window_w < TOOLS_AUTO_COLLAPSE_BELOW);

    let widths = |n: bool, s: bool, t: bool| {
        (
            if n {
                NOTIFICATIONS_COLLAPSED
            } else {
                NOTIFICATIONS_EXPANDED
            },
            if s {
                SESSIONS_COLLAPSED
            } else {
                SESSIONS_EXPANDED
            },
            if t { TOOLS_COLLAPSED } else { TOOLS_EXPANDED },
        )
    };
    let chat = |(n, s, t): (u32, u32, u32)| window_w.saturating_sub(n + s + t);

    if chat(widths(notifications, sessions, tools)) < CHAT_MIN
        && manual.notifications.manual == Manual::Auto
    {
        notifications = true;
    }
    if chat(widths(notifications, sessions, tools)) < CHAT_MIN
        && manual.tools.manual == Manual::Auto
    {
        tools = true;
    }

    let (notifications_w, sessions_w, tools_w) = widths(notifications, sessions, tools);
    LayoutState {
        notifications_collapsed: notifications,
        sessions_collapsed: sessions,
        tools_collapsed: tools,
        notifications_w,
        sessions_w,
        tools_w,
        chat_w: chat((notifications_w, sessions_w, tools_w)),
    }
}

fn collapsed_by(manual: Manual, auto_collapsed: bool) -> bool {
    match manual {
        Manual::Collapsed => true,
        Manual::Expanded => false,
        Manual::Auto => auto_collapsed,
    }
}
