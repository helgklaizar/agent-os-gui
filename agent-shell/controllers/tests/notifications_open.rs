//! "Open chat": an event with a session opens the session and the tab from `Event.tab`, the session's unread events are marked
//! read; an event without a session is dismissed with "OK".

mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::notif_types::{EventItem, EventsFilter};
use controllers::{BusEvent, CoreCommand, Tab};
use notif_common::*;

fn with_tab(mut e: EventItem, tab: &str) -> EventItem {
    e.tab = tab.to_string();
    e
}

fn open_session(id: i64, tab: Tab) -> BusEvent {
    BusEvent::OpenSession {
        session_id: id,
        tab,
    }
}

fn read_session(id: i64) -> CoreCommand {
    CoreCommand::MarkEventsRead(EventsFilter {
        session_id: Some(id),
        ..EventsFilter::default()
    })
}

#[test]
fn notifications_open_chat_selects_session_and_tab() {
    let mut r = rig(vec![ev(1, "ai", "agent", "reply", Some(7), true)]);
    let fx = r.send(NotifMsg::Open(1));
    assert_eq!(
        published(&fx),
        vec![open_session(7, Tab::Chat)],
        "an event with a session produces OpenSession"
    );
    assert_eq!(
        effects_of_commands(&fx),
        vec![read_session(7)],
        "the session's unread events are marked in the core"
    );
}

#[test]
fn open_chat_tab_comes_from_event_tab_with_chat_default() {
    for (field, want) in [
        ("chat", Tab::Chat),
        ("", Tab::Chat),
        ("browser", Tab::Browser),
        ("terminal", Tab::Terminal),
        ("settings", Tab::SessionSettings),
        ("unknown-tab", Tab::Chat),
    ] {
        let mut r = rig(vec![with_tab(
            ev(1, "ai", "agent", "x", Some(7), true),
            field,
        )]);
        let fx = r.send(NotifMsg::Open(1));
        assert_eq!(
            published(&fx),
            vec![open_session(7, want)],
            "Event.tab = {field:?}"
        );
    }
}

#[test]
fn open_chat_marks_all_unread_of_that_session_and_only_it() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "a", Some(7), true),
        ev(2, "task", "done", "b", Some(7), true),
        ev(3, "ai", "agent", "c", Some(9), true),
        ev(4, "msg", "sys", "d", None, true),
    ]);
    r.send(NotifMsg::Open(1));
    assert_eq!(r.card(1).map(|c| c.unread), Some(false));
    assert_eq!(
        r.card(2).map(|c| c.unread),
        Some(false),
        "another event of the same session"
    );
    assert_eq!(
        r.card(3).map(|c| c.unread),
        Some(true),
        "a foreign session is untouched"
    );
    assert_eq!(
        r.card(4).map(|c| c.unread),
        Some(true),
        "the system event is untouched"
    );
    assert_eq!(r.view().total_unread, 2);
}

#[test]
fn notifications_open_without_session_only_marks_read() {
    let mut r = rig(vec![
        ev(1, "msg", "sys", "battery", None, true),
        ev(2, "msg", "sys", "other", None, true),
    ]);
    let fx = r.send(NotifMsg::Open(1));
    assert!(published(&fx).is_empty(), "no session: nothing opens");
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::MarkEventRead { id: 1 }]
    );
    assert_eq!(r.card(1).map(|c| c.unread), Some(false));
    assert_eq!(
        r.card(2).map(|c| c.unread),
        Some(true),
        "the neighbouring system event stays"
    );
    assert_eq!(r.view().total_unread, 1);
}

#[test]
fn open_unknown_event_does_nothing() {
    let mut r = rig(vec![ev(1, "ai", "agent", "x", Some(7), true)]);
    let fx = r.send(NotifMsg::Open(999));
    assert!(fx.is_empty());
    assert_eq!(r.view().total_unread, 1);
}

#[test]
fn opening_a_session_from_elsewhere_marks_its_events_once() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "a", Some(7), true),
        ev(2, "ai", "agent", "b", Some(9), true),
    ]);
    let fx = r.send(NotifMsg::Bus(open_session(7, Tab::Chat)));
    assert_eq!(
        effects_of_commands(&fx),
        vec![read_session(7)],
        "opened from the tree: the session's events are read"
    );
    assert_eq!(r.view().total_unread, 1);
    let again = r.send(NotifMsg::Bus(open_session(7, Tab::Chat)));
    assert!(
        effects_of_commands(&again).is_empty(),
        "reopening without unread events sends no command"
    );
}

#[test]
fn card_open_then_own_bus_event_does_not_duplicate_the_command() {
    let mut r = rig(vec![ev(1, "ai", "agent", "a", Some(7), true)]);
    let fx = r.send(NotifMsg::Open(1));
    assert_eq!(effects_of_commands(&fx).len(), 1);
    let echo = r.send(NotifMsg::Bus(open_session(7, Tab::Chat)));
    assert!(
        effects_of_commands(&echo).is_empty(),
        "OpenSession came back from the bus: no second command"
    );
}
