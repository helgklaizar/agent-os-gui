//! S3-03, S3-06, S3-08: tree assembly, selection by id, deletion, feed events.

mod tree_common;

use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::i18n::tr;
use controllers::tree_types::{FeedEvent, TreeFrame};
use controllers::tree_view::RowKind;
use controllers::{BusEvent, Controller, Effect};
use tree_common::*;

#[test]
fn tree_builds_nested() {
    let c = loaded();
    let v = c.view();
    assert_eq!(
        nodes(&v),
        vec![
            f(1),
            f(2),
            s(7),
            s(8),
            s(9),
            s(10),
            s(5),
            f(3),
            s(6),
            s(3),
            s(4)
        ],
        "folders, then sessions; threads under the parent"
    );
    let depth = |n| row(&v, n).map(|r| (r.depth, r.indent, r.kind));
    assert_eq!(depth(f(1)), Some((0, 0, RowKind::Folder)));
    assert_eq!(depth(f(2)), Some((1, 16, RowKind::Folder)));
    assert_eq!(depth(s(7)), Some((2, 32, RowKind::Session)));
    assert_eq!(depth(s(8)), Some((3, 48, RowKind::Thread)));
    assert_eq!(depth(s(3)), Some((0, 0, RowKind::Session)));
    assert_eq!(row(&v, f(1)).and_then(|r| r.expanded), Some(true));
    assert_eq!(
        row(&v, s(7)).and_then(|r| r.expanded),
        None,
        "a session has no expansion"
    );
    assert!(!v.empty);
    assert_eq!(v.new_session, tr("tree-new-session"));
}

#[test]
fn tree_snapshot_replaces_old_state() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, "One")],
        },
    );
    assert_eq!(
        nodes(&c.view()),
        vec![s(1)],
        "a snapshot replaces, not supplements"
    );
}

#[test]
fn tree_frames_are_idempotent_upserts() {
    let mut c = loaded();
    let before = c.view().rows.len();
    let created = session(20, Some(2), "New");
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::SessionCreated(created.clone())),
    );
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionCreated(created)));
    let v = c.view();
    assert_eq!(
        v.rows.len(),
        before + 1,
        "a repeated frame does not duplicate the row"
    );
    assert_eq!(
        row(&v, s(20)).map(|r| r.depth),
        Some(2),
        "in the right folder"
    );
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(999)));
    assert_eq!(
        c.view().rows.len(),
        before + 1,
        "deleting an unknown id is harmless"
    );
}

#[test]
fn tree_create_keeps_selection() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(6));
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::SessionCreated(session(20, Some(1), "New"))),
    );
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderCreated(folder(
            30,
            None,
            "New folder",
            None,
        ))),
    );
    let v = c.view();
    assert_eq!(v.selected, Some(6), "selection did not drift");
    assert!(row(&v, s(6)).is_some_and(|r| r.selected));
}

#[test]
fn tree_selection_by_id() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(6));
    let index_before = c.view().rows.iter().position(|r| r.node == s(6));
    let fx = send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(5)));
    let v = c.view();
    assert_eq!(
        v.selected,
        Some(6),
        "deleting a session left of the selected one does not change selection"
    );
    let selected: Vec<_> = v.rows.iter().filter(|r| r.selected).collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].node,
        s(6),
        "the same row is selected, not the same index"
    );
    assert_eq!(selected[0].name, "Vacation");
    assert_ne!(
        v.rows.iter().position(|r| r.node == s(6)),
        index_before,
        "the index shifted"
    );
    assert!(
        !fx.contains(&open_session(5))
            && !fx
                .iter()
                .any(|e| matches!(e, Effect::Publish(BusEvent::OpenSession { .. }))),
        "the session was not reopened: {fx:?}"
    );
}

#[test]
fn tree_selection_survives_snapshot_rebuild() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(7));
    let (folders, mut sessions) = fixture();
    sessions.reverse();
    send(&mut c, TreeMsg::Snapshot { folders, sessions });
    assert_eq!(c.view().selected, Some(7));
    assert!(row(&c.view(), s(7)).is_some_and(|r| r.selected));
}

#[test]
fn tree_select_opens_session_in_chat() {
    let mut c = loaded();
    let fx = send(&mut c, TreeMsg::Select(5));
    assert!(fx.contains(&open_session(5)), "{fx:?}");
    assert_eq!(c.view().selected, Some(5));
    assert!(
        send(&mut c, TreeMsg::Select(999)).is_empty(),
        "an unknown id is ignored"
    );
    assert_eq!(c.view().selected, Some(5));
}

#[test]
fn tree_delete_selected_picks_neighbour() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(5));
    let fx = send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(5)));
    assert_eq!(
        c.view().selected,
        Some(6),
        "the neighbour after the deleted one"
    );
    assert!(fx.contains(&open_session(6)), "{fx:?}");
    send(&mut c, TreeMsg::Select(4));
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(4)));
    assert_eq!(c.view().selected, Some(3), "for the last one, the previous");
}

#[test]
fn tree_delete_parent_with_selected_thread_picks_neighbour() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Select(9));
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(7)));
    let v = c.view();
    assert!(row(&v, s(9)).is_none(), "the thread went with its parent");
    assert_eq!(
        v.selected,
        Some(5),
        "a neighbouring session is selected, not empty space"
    );
}

#[test]
fn tree_delete_last_session_shows_create_hint() {
    let mut c = SessionTreeController::new();
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, "One")],
        },
    );
    send(&mut c, TreeMsg::Select(1));
    assert_eq!(c.view().empty_hint, None);
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(1)));
    let v = c.view();
    assert_eq!(v.selected, None);
    assert!(v.empty);
    assert_eq!(v.empty_hint, Some(tr("chat-empty")));
    assert_eq!(tr("chat-empty"), "Create a session on the left");
}

#[test]
fn tree_delete_keeps_system_events() {
    let mut c = loaded();
    let feed = vec![
        FeedEvent {
            id: 1,
            text: "system".into(),
            session_id: None,
        },
        FeedEvent {
            id: 2,
            text: "reply".into(),
            session_id: Some(5),
        },
        FeedEvent {
            id: 3,
            text: "thread".into(),
            session_id: Some(8),
        },
        FeedEvent {
            id: 4,
            text: "other".into(),
            session_id: Some(6),
        },
    ];
    send(&mut c, TreeMsg::Feed(feed));
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(5)));
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionDeleted(7)));
    let v = c.view();
    assert_eq!(v.feed.len(), 4, "no event was lost: {:?}", v.feed);
    let sid = |id| v.feed.iter().find(|e| e.id == id).map(|e| e.session_id);
    assert_eq!(sid(1), Some(None), "the system event is in place");
    assert_eq!(
        sid(2),
        Some(None),
        "the deleted session's event is left without session_id"
    );
    assert_eq!(sid(3), Some(None), "and its thread's event");
    assert_eq!(sid(4), Some(Some(6)), "others are untouched");
}
