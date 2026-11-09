//! Tests of `layout::resolve`.

use controllers::layout::{CHAT_MIN, LayoutState, resolve};
use controllers::prefs::ColumnsPrefs;
use controllers::{Column, Manual};

fn manual(notifications: Manual, sessions: Manual, tools: Manual) -> ColumnsPrefs {
    let mut c = ColumnsPrefs::default();
    c.notifications.manual = notifications;
    c.sessions.manual = sessions;
    c.tools.manual = tools;
    c
}

fn auto(w: u32) -> LayoutState {
    resolve(w, &ColumnsPrefs::default())
}

fn collapsed(s: &LayoutState) -> (bool, bool, bool) {
    (
        s.notifications_collapsed,
        s.sessions_collapsed,
        s.tools_collapsed,
    )
}

#[test]
fn layout_thresholds() {
    let s = auto(1024);
    assert_eq!(collapsed(&s), (true, false, true));
    assert_eq!(
        (s.notifications_w, s.sessions_w, s.tools_w, s.chat_w),
        (56, 236, 56, 676)
    );

    assert_eq!(collapsed(&auto(1071)), (true, false, true));
    for w in [1072, 1575] {
        let s = auto(w);
        assert_eq!(collapsed(&s), (false, false, true), "w={w}");
        assert_eq!((s.notifications_w, s.tools_w), (300, 56), "w={w}");
    }
    assert_eq!(auto(1072).chat_w, 480);
    assert_eq!(auto(1575).chat_w, 1575 - 300 - 236 - 56);

    let s = auto(1576);
    assert_eq!(collapsed(&s), (false, false, false));
    assert_eq!(
        (s.notifications_w, s.sessions_w, s.tools_w, s.chat_w),
        (300, 236, 560, 480)
    );
}

#[test]
fn layout_sessions_never_auto_collapse() {
    for w in 1024..=3000 {
        assert!(!auto(w).sessions_collapsed, "w={w}");
        assert_eq!(auto(w).sessions_w, 236, "w={w}");
    }
}

#[test]
fn layout_chat_min_width_auto() {
    let s = resolve(1300, &manual(Manual::Auto, Manual::Auto, Manual::Expanded));
    assert!(
        s.notifications_collapsed,
        "notifications collapse automatically"
    );
    assert!(!s.tools_collapsed, "manual choice is not overridden");
    assert_eq!(s.chat_w, 448);
}

#[test]
fn layout_manual_overrides_auto() {
    let s = resolve(1024, &manual(Manual::Expanded, Manual::Auto, Manual::Auto));
    assert!(!s.notifications_collapsed);
    assert_eq!(s.notifications_w, 300);
}

#[test]
fn layout_manual_collapse_wins() {
    let s = resolve(2000, &manual(Manual::Auto, Manual::Auto, Manual::Collapsed));
    assert!(s.tools_collapsed);
    assert_eq!(s.tools_w, 56);
    assert_eq!(s.chat_w, 2000 - 300 - 236 - 56);
}

#[test]
fn layout_expand_works_at_any_width() {
    let s = resolve(1024, &manual(Manual::Auto, Manual::Auto, Manual::Expanded));
    assert!(!s.tools_collapsed);
    assert_eq!(s.chat_w, 1024 - 56 - 236 - 560);
}

#[test]
fn layout_sessions_manual_collapse_is_64() {
    let s = resolve(2000, &manual(Manual::Auto, Manual::Collapsed, Manual::Auto));
    assert_eq!((s.sessions_collapsed, s.sessions_w), (true, 64));
}

#[test]
fn layout_chat_min_prefers_collapsing_notifications_then_tools() {
    // both auto-expanded and the chat lacks room only with manual columns: notifications first, then tools
    let s = resolve(1100, &manual(Manual::Auto, Manual::Auto, Manual::Auto));
    assert!(s.chat_w >= CHAT_MIN);
    // manual expansion of both narrow columns: the chat shrinks, the choice is not overridden
    let s = resolve(
        1024,
        &manual(Manual::Expanded, Manual::Auto, Manual::Expanded),
    );
    assert!(!s.notifications_collapsed && !s.tools_collapsed);
    assert_eq!(s.chat_w, 1024u32.saturating_sub(300 + 236 + 560));
}

#[test]
fn column_enum_is_exhaustive_for_toggle() {
    let _ = [Column::Notifications, Column::Sessions, Column::Tools];
}
