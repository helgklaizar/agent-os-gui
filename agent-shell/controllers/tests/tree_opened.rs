//! S5-07: a session opened outside the tree (feed card): the tree selects it and clears unread, but does not publish `OpenSession` again.

mod tree_common;

use controllers::controllers::session_tree::TreeMsg;
use controllers::{BusEvent, Controller, CoreCommand, Effect};
use tree_common::*;

#[test]
fn tree_opened_from_bus_selects_without_republishing() {
    let mut c = loaded();
    let fx = send(&mut c, TreeMsg::Opened(6));
    assert_eq!(c.view().selected, Some(6), "session selected");
    assert_eq!(c.view().focus, Some(s(6)));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Publish(BusEvent::OpenSession { .. }))),
        "re-publishing would loop the bus: {fx:?}"
    );
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::MarkRead { id: 6 }],
        "\"Vacation\" had 2 unread"
    );
    let v = c.view();
    assert_eq!(
        row(&v, s(6)).map(|r| r.badge.clone()),
        Some(None),
        "badge cleared"
    );
}

#[test]
fn tree_opened_unknown_session_changes_nothing() {
    let mut c = loaded();
    let fx = send(&mut c, TreeMsg::Opened(999));
    assert!(fx.is_empty());
    assert_eq!(c.view().selected, None);
}

#[test]
fn tree_opened_same_session_twice_marks_once() {
    let mut c = loaded();
    send(&mut c, TreeMsg::Opened(6));
    let again = send(&mut c, TreeMsg::Opened(6));
    assert!(commands(&again).is_empty(), "no unread left");
}
