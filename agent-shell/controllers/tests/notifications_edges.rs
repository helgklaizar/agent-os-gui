//! Feed edge cases found by mutation testing (`cargo mutants --in-diff`): repeated selection, reading an already read event,
//! the "read" filter by subtype, deleting a session with a child, the badge at the 99/100 boundary, kind icons.

mod notif_common;

use controllers::CoreCommand;
use controllers::controllers::notifications::NotifMsg;
use controllers::notif_render::badge;
use controllers::notif_types::{EventKind, EventsFilter};
use controllers::notif_view::Selection;
use controllers::tree_types::{SessionItem, TreeFrame};
use notif_common::*;

fn session(id: i64, parent: Option<i64>) -> SessionItem {
    SessionItem {
        id,
        parent_id: parent,
        name: format!("s{id}"),
        ..SessionItem::default()
    }
}

#[test]
fn notifications_select_other_kind_or_sub_replaces_selection() {
    let mut r = rig(vec![ev(1, "ai", "agent", "a", None, true)]);
    r.send(NotifMsg::SelectKind(EventKind::Ai));
    r.send(NotifMsg::SelectKind(EventKind::Task));
    assert_eq!(r.view().selection, Selection::Kind(EventKind::Task));

    r.send(NotifMsg::SelectSub(EventKind::Ai, "watch".into()));
    r.send(NotifMsg::SelectSub(EventKind::Ai, "agent".into()));
    assert_eq!(
        r.view().selection,
        Selection::Sub(EventKind::Ai, "agent".into()),
        "another subtype of the same kind replaces the selection"
    );
    r.send(NotifMsg::SelectSub(EventKind::Msg, "agent".into()));
    assert_eq!(
        r.view().selection,
        Selection::Sub(EventKind::Msg, "agent".into()),
        "the same subtype of another kind is a different selection"
    );
}

#[test]
fn notifications_read_of_already_read_event_keeps_counts() {
    let mut r = rig(vec![
        ev(4, "msg", "wa", "m1", None, true),
        ev(5, "msg", "wa", "m2", None, false),
    ]);
    r.send(NotifMsg::Read(vec![5]));
    assert_eq!(
        r.view().total_unread,
        1,
        "a read event does not decrease the counter"
    );
    assert_eq!(r.kind(EventKind::Msg).badge.count, 1);
}

#[test]
fn notifications_mark_read_by_sub_touches_only_that_sub() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "a", None, true),
        ev(2, "msg", "wa", "w", None, true),
        ev(3, "msg", "sys", "s", None, true),
    ]);
    let filter = EventsFilter {
        kind: None,
        sub: Some("wa".into()),
        session_id: None,
    };
    let fx = r.send(NotifMsg::MarkRead(filter.clone()));
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::MarkEventsRead(filter)]
    );
    assert!(!r.card(2).is_some_and(|c| c.unread), "wa is read");
    assert!(r.card(1).is_some_and(|c| c.unread));
    assert!(r.card(3).is_some_and(|c| c.unread));
    assert_eq!(r.view().total_unread, 2);
}

#[test]
fn notifications_open_read_event_without_session_sends_nothing() {
    let mut r = rig(vec![
        ev(1, "msg", "sys", "read", None, false),
        ev(2, "msg", "sys", "new", None, true),
    ]);
    let fx = r.send(NotifMsg::Open(1));
    assert!(effects_of_commands(&fx).is_empty(), "no command needed");
    let fx = r.send(NotifMsg::Open(2));
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::MarkEventRead { id: 2 }]
    );
}

#[test]
fn notifications_session_delete_detaches_only_its_branch() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "root", Some(7), true),
        ev(2, "ai", "agent", "child", Some(8), true),
        ev(3, "ai", "agent", "foreign", Some(9), true),
    ]);
    r.send(NotifMsg::Sessions(vec![
        session(7, None),
        session(8, Some(7)),
        session(9, None),
    ]));
    r.send(NotifMsg::Tree(TreeFrame::SessionDeleted(7)));
    assert_eq!(r.card(1).and_then(|c| c.session_id), None);
    assert_eq!(r.card(2).and_then(|c| c.session_id), None);
    assert_eq!(r.card(3).and_then(|c| c.session_id), Some(9));
    assert!(r.card(3).is_some_and(|c| c.session_label.is_some()));
}

#[test]
fn notifications_badge_boundaries() {
    assert_eq!(badge("x", 0).text, None);
    assert_eq!(badge("x", 1).text.as_deref(), Some("1"));
    assert_eq!(badge("x", 99).text.as_deref(), Some("99"));
    assert_eq!(badge("x", 100).text.as_deref(), Some("99+"));
}

#[test]
fn notifications_kind_icons_are_distinct_symbolic_names() {
    let r = rig(vec![]);
    let icons: Vec<&str> = [EventKind::Ai, EventKind::Task, EventKind::Msg]
        .iter()
        .map(|k| r.kind(*k).icon)
        .collect();
    assert!(icons.iter().all(|i| i.ends_with("-symbolic")), "{icons:?}");
    assert!(icons[0] != icons[1] && icons[1] != icons[2] && icons[0] != icons[2]);
}
