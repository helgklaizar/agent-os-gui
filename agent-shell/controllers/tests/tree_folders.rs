//! S3-06, S3-08: folder expansion, orphans, keyboard, focus, a11y names.

mod tree_common;

use controllers::Controller;
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::i18n::tr;
use controllers::tree_types::TreeFrame;
use controllers::tree_view::NavKey;
use tree_common::*;

#[test]
fn folder_toggle_state_client_side() {
    let mut c = loaded();
    let fx = send(&mut c, TreeMsg::ToggleFolder(1));
    assert!(
        !has_core_command(&fx),
        "no command goes to the core: {fx:?}"
    );
    let v = c.view();
    assert_eq!(row(&v, f(1)).and_then(|r| r.expanded), Some(false));
    assert_eq!(
        nodes(&v),
        vec![f(1), f(3), s(6), s(3), s(4)],
        "contents of a collapsed folder are hidden"
    );
    let (folders, sessions) = fixture();
    send(&mut c, TreeMsg::Snapshot { folders, sessions });
    assert_eq!(
        row(&c.view(), f(1)).and_then(|r| r.expanded),
        Some(false),
        "a rebuild does not re-expand"
    );
    send(&mut c, TreeMsg::ToggleFolder(1));
    assert_eq!(c.view().rows.len(), 11);
}

#[test]
fn folder_open_flag_sets_initial_state() {
    let mut c = SessionTreeController::new();
    let mut closed = folder(1, None, "Closed", None);
    closed.open = false;
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![closed],
            sessions: vec![session(2, Some(1), "Inside")],
        },
    );
    let v = c.view();
    assert_eq!(nodes(&v), vec![f(1)]);
    assert_eq!(row(&v, f(1)).and_then(|r| r.expanded), Some(false));
}

#[test]
fn selected_session_in_collapsed_folder_stays_selected() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(7));
    send(&mut c, TreeMsg::ToggleFolder(1));
    assert_eq!(
        c.view().selected,
        Some(7),
        "selection is held by id, the row is hidden"
    );
}

#[test]
fn orphan_nodes_show_at_root_until_parent_frame_arrives() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Frame(TreeFrame::FolderDeleted(1)));
    let v = c.view();
    assert_eq!(
        row(&v, f(2)).map(|r| r.depth),
        Some(0),
        "a subfolder without a parent goes to the root"
    );
    assert_eq!(row(&v, s(5)).map(|r| r.depth), Some(0));
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderUpdated(folder(2, None, "Projects", None))),
    );
    assert_eq!(row(&c.view(), f(2)).map(|r| r.depth), Some(0));
}

#[test]
fn tree_keyboard_arrows_walk_visible_rows() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Key(NavKey::Down));
    assert_eq!(
        c.view().focus,
        Some(f(1)),
        "the first Down goes to the first row"
    );
    send(&mut c, TreeMsg::Key(NavKey::Down));
    assert_eq!(c.view().focus, Some(f(2)));
    send(&mut c, TreeMsg::Key(NavKey::Up));
    assert_eq!(c.view().focus, Some(f(1)));
    send(&mut c, TreeMsg::Key(NavKey::Up));
    assert_eq!(c.view().focus, Some(f(1)), "does not go above the first");
    send(&mut c, TreeMsg::Key(NavKey::End));
    assert_eq!(c.view().focus, Some(s(4)));
    send(&mut c, TreeMsg::Key(NavKey::Down));
    assert_eq!(c.view().focus, Some(s(4)), "does not go below the last");
    send(&mut c, TreeMsg::Key(NavKey::Home));
    assert_eq!(c.view().focus, Some(f(1)));
    assert!(row(&c.view(), f(1)).is_some_and(|r| r.focused));
}

#[test]
fn tree_keyboard_right_left_expand_collapse() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Focus(f(1)));
    let fx = send(&mut c, TreeMsg::Key(NavKey::Left));
    assert!(!has_core_command(&fx));
    assert_eq!(
        row(&c.view(), f(1)).and_then(|r| r.expanded),
        Some(false),
        "Left collapses an expanded folder"
    );
    send(&mut c, TreeMsg::Key(NavKey::Right));
    assert_eq!(
        row(&c.view(), f(1)).and_then(|r| r.expanded),
        Some(true),
        "Right expands a collapsed one"
    );
    send(&mut c, TreeMsg::Key(NavKey::Right));
    assert_eq!(
        c.view().focus,
        Some(f(2)),
        "Right on an expanded one goes to the first child"
    );
    send(&mut c, TreeMsg::Focus(s(5)));
    send(&mut c, TreeMsg::Key(NavKey::Left));
    assert_eq!(
        c.view().focus,
        Some(f(1)),
        "Left on a session goes to the parent folder"
    );
    send(&mut c, TreeMsg::Focus(s(8)));
    send(&mut c, TreeMsg::Key(NavKey::Left));
    assert_eq!(
        c.view().focus,
        Some(s(7)),
        "Left on a thread goes to the session"
    );
    send(&mut c, TreeMsg::Focus(s(3)));
    send(&mut c, TreeMsg::Key(NavKey::Left));
    assert_eq!(
        c.view().focus,
        Some(s(3)),
        "Left on a root session does nothing"
    );
}

#[test]
fn tree_keyboard_enter_opens_session_or_toggles_folder() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Focus(s(6)));
    let fx = send(&mut c, TreeMsg::Key(NavKey::Enter));
    assert!(fx.contains(&open_session(6)), "{fx:?}");
    assert_eq!(c.view().selected, Some(6));
    send(&mut c, TreeMsg::Focus(f(3)));
    let fx = send(&mut c, TreeMsg::Key(NavKey::Enter));
    assert!(fx.is_empty(), "Enter on a folder only expands it: {fx:?}");
    assert_eq!(row(&c.view(), f(3)).and_then(|r| r.expanded), Some(false));
    assert_eq!(
        c.view().selected,
        Some(6),
        "a folder does not become the selected session"
    );
}

#[test]
fn tree_focus_survives_when_row_disappears() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Focus(s(5)));
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(5)));
    let focus = c.view().focus;
    assert!(
        focus.is_some_and(|n| row(&c.view(), n).is_some()),
        "focus stays on an existing row: {focus:?}"
    );
}

#[test]
fn a11y_names_carry_name_role_state_and_unread() {
    let c = loaded();
    let v = c.view();
    let folder_name = row(&v, f(1))
        .map(|r| r.a11y_name.clone())
        .unwrap_or_default();
    assert!(
        folder_name.contains("Work") && folder_name.contains(&tr("tree-a11y-folder")),
        "{folder_name}"
    );
    assert!(
        folder_name.contains(&tr("tree-a11y-expanded")),
        "{folder_name}"
    );
    let session = row(&v, s(6))
        .map(|r| r.a11y_name.clone())
        .unwrap_or_default();
    assert!(
        session.contains("Vacation") && session.contains(&tr("tree-a11y-session")),
        "{session}"
    );
    assert!(session.contains('2'), "unread count: {session}");
    let thread = row(&v, s(8))
        .map(|r| r.a11y_name.clone())
        .unwrap_or_default();
    assert!(thread.contains(&tr("tree-a11y-thread")), "{thread}");
    let quiet = row(&v, s(3))
        .map(|r| r.a11y_name.clone())
        .unwrap_or_default();
    assert!(
        !quiet.contains(&tr("tree-a11y-unread")),
        "no unread line without unread: {quiet}"
    );
}
