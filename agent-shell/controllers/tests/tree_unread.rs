//! Badge from `unread`, reset on open, collapsed column, strings from `en.ftl`.

mod tree_common;

use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::i18n::{EN_FTL, tr};
use controllers::tree_types::TreeFrame;
use controllers::{Controller, CoreCommand, Effect};
use tree_common::*;

fn badge(c: &SessionTreeController, id: i64) -> Option<String> {
    row(&c.view(), s(id)).and_then(|r| r.badge.clone())
}

#[test]
fn unread_badge_from_session() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 6,
            unread: 3,
        }),
    );
    assert_eq!(badge(&c, 6).as_deref(), Some("3"));
    assert_eq!(row(&c.view(), s(6)).map(|r| r.unread), Some(3));
    let fx = send(&mut c, TreeMsg::Select(6));
    assert!(
        fx.contains(&Effect::Command(CoreCommand::MarkRead { id: 6 })),
        "reset command: {fx:?}"
    );
    assert_eq!(badge(&c, 6), None, "badge gone");
    assert_eq!(row(&c.view(), s(6)).map(|r| r.unread), Some(0));
    assert_eq!(
        badge(&c, 3),
        None,
        "a session with nothing unread has no badge"
    );
}

#[test]
fn unread_select_without_unread_sends_no_reset() {
    let mut c = loaded();
    let fx = send(&mut c, TreeMsg::Select(5));
    assert!(
        fx.iter().all(|e| !matches!(e, Effect::Command(_))),
        "{fx:?}"
    );
}

#[test]
fn unread_badge_caps_at_99_plus() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 5,
            unread: 99,
        }),
    );
    assert_eq!(badge(&c, 5).as_deref(), Some("99"));
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 5,
            unread: 100,
        }),
    );
    assert_eq!(badge(&c, 5).as_deref(), Some("99+"));
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 5,
            unread: -4,
        }),
    );
    assert_eq!(badge(&c, 5), None, "a negative number gives no badge");
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 999,
            unread: 1,
        }),
    );
}

#[test]
fn unread_for_open_session_is_reset_at_once() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(5));
    let fx = send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 5,
            unread: 1,
        }),
    );
    assert!(
        fx.contains(&Effect::Command(CoreCommand::MarkRead { id: 5 })),
        "a reply in the open session is read at once: {fx:?}"
    );
    assert_eq!(badge(&c, 5), None);
    let fx = send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 3,
            unread: 2,
        }),
    );
    assert!(
        fx.is_empty(),
        "in an inactive session the badge grows: {fx:?}"
    );
    assert_eq!(badge(&c, 3).as_deref(), Some("2"));
}

#[test]
fn unread_updates_via_session_frames() {
    let mut c = loaded();
    let mut updated = session(3, None, "Loose");
    updated.unread = 4;
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionUpdated(updated)));
    assert_eq!(badge(&c, 3).as_deref(), Some("4"));
}

#[test]
fn collapsed_column_shows_unread() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::Unread {
            session_id: 7,
            unread: 3,
        }),
    );
    assert!(c.view().strip.is_none(), "no strip in an expanded column");
    send(&mut c, TreeMsg::Collapsed(true));
    let v = c.view();
    assert!(v.collapsed);
    let strip = v.strip.expect("collapsed column strip");
    assert_eq!(strip.total_unread, 5);
    assert_eq!(
        strip.badge.as_deref(),
        Some("5"),
        "indicator on the column icon"
    );
    let items: Vec<_> = strip
        .items
        .iter()
        .map(|i| (i.session_id, i.badge.as_str()))
        .collect();
    assert_eq!(
        items,
        vec![(7, "3"), (6, "2")],
        "icons of sessions with unread"
    );
    assert!(strip.a11y_name.contains('5'), "{}", strip.a11y_name);
    send(&mut c, TreeMsg::Select(7));
    let strip = c.view().strip.expect("strip");
    assert_eq!(
        strip.badge.as_deref(),
        Some("2"),
        "the counter is recomputed after open"
    );
    send(&mut c, TreeMsg::Collapsed(false));
    assert!(c.view().strip.is_none());
}

#[test]
fn collapsed_column_without_unread_has_no_badge() {
    let mut c = SessionTreeController::new();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, "Quiet")],
        },
    );
    send(&mut c, TreeMsg::Collapsed(true));
    let strip = c.view().strip.expect("strip");
    assert_eq!(strip.badge, None);
    assert!(strip.items.is_empty());
}

#[test]
fn collapsed_column_counts_unread_in_collapsed_folders() {
    let mut c = loaded();
    send(&mut c, TreeMsg::ToggleFolder(3));
    send(&mut c, TreeMsg::Collapsed(true));
    assert_eq!(
        c.view().strip.map(|s| s.total_unread),
        Some(2),
        "a collapsed folder does not hide badges"
    );
}

#[test]
fn folder_toggle_does_not_touch_unread() {
    let mut c = loaded();
    send(&mut c, TreeMsg::ToggleFolder(3));
    assert_eq!(row(&c.view(), f(3)).map(|r| r.unread), Some(0));
}

// ---------- strings from en.ftl ----------

fn ftl_keys() -> Vec<String> {
    EN_FTL
        .lines()
        .filter(|l| !l.starts_with(['#', ' ', '\t', '.', '[', '*']))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
        .filter(|k| !k.is_empty())
        .collect()
}

#[test]
fn i18n_tree_strings_from_ftl() {
    let keys = ftl_keys();
    for key in [
        "tree-new-session",
        "tree-new-folder",
        "chat-empty",
        "dialog-create",
        "dialog-delete",
        "dialog-no-folder",
        "dialog-persona-inherit",
        "delete-session-title",
        "delete-folder-title",
        "delete-folder-lines",
        "tree-ctx-isolated",
    ] {
        assert!(keys.iter().any(|k| k == key), "en.ftl has no `{key}`");
    }
    let c = loaded();
    assert_eq!(c.view().new_session, tr("tree-new-session"));
    assert_eq!(c.view().new_folder, tr("tree-new-folder"));
    assert_eq!(tr("tree-new-session"), "New session");
    assert_eq!(tr("tree-new-folder"), "New folder");
    assert!(!tr("delete-session-title").starts_with('⟦'), "key found");
}
