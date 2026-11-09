//! S5-08: a 4 s toast for a reply in an inactive session; no toast for the active session, the event is read at once.

mod notif_common;

use std::time::Duration;

use controllers::controllers::notifications::NotifMsg;
use controllers::controllers::shell::{ShellController, ShellMsg, TOAST_DURATION};
use controllers::notif_types::EventItem;
use controllers::prefs::UiFile;
use controllers::tree_types::SessionItem;
use controllers::{BusEvent, Controller, CoreCommand, Effect, Tab};
use notif_common::*;

fn open(id: i64) -> NotifMsg {
    NotifMsg::Bus(BusEvent::OpenSession {
        session_id: id,
        tab: Tab::Chat,
    })
}

fn toasts(fx: &[Effect]) -> Vec<(i64, String)> {
    published(fx)
        .into_iter()
        .filter_map(|b| match b {
            BusEvent::ShowToast { session_id, text } => Some((session_id, text)),
            _ => None,
        })
        .collect()
}

fn named(r: &mut Rig) {
    r.send(NotifMsg::Sessions(vec![
        SessionItem {
            id: 9,
            name: "Release".into(),
            ..SessionItem::default()
        },
        SessionItem {
            id: 7,
            name: "Notes".into(),
            ..SessionItem::default()
        },
    ]));
}

fn reply(id: i64, session: i64) -> EventItem {
    ev(id, "ai", "agent", "reply ready", Some(session), true)
}

#[test]
fn notifications_toast_only_for_inactive_session() {
    let mut r = rig(Vec::new());
    named(&mut r);
    r.send(open(7));
    let fx = r.send(NotifMsg::Created(reply(1, 9)));
    assert_eq!(
        toasts(&fx),
        vec![(9, "Release: reply ready".to_string())],
        "a reply in an inactive session produces a toast"
    );
    assert_eq!(r.view().total_unread, 1, "the event in the feed is unread");

    let fx = r.send(NotifMsg::Created(reply(2, 7)));
    assert!(toasts(&fx).is_empty(), "no toast for the active session");
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::MarkEventRead { id: 2 }],
        "and the event is read at once"
    );
    assert_eq!(r.view().total_unread, 1, "the counter did not grow");
    assert_eq!(r.card(2).map(|c| c.unread), Some(false));
}

#[test]
fn toast_needs_a_new_unread_ai_event_with_a_session() {
    let mut r = rig(Vec::new());
    let cases = [
        ("session task", ev(1, "task", "done", "x", Some(9), true)),
        ("message", ev(2, "msg", "wa", "x", Some(9), true)),
        ("system AI event", ev(3, "ai", "claude", "x", None, true)),
        ("already read", ev(4, "ai", "agent", "x", Some(9), false)),
    ];
    for (name, e) in cases {
        let fx = r.send(NotifMsg::Created(e));
        assert!(toasts(&fx).is_empty(), "{name}");
    }
    let fx = r.send(NotifMsg::Created(ev(5, "ai", "watch", "x", Some(9), true)));
    assert_eq!(toasts(&fx).len(), 1, "any AI event of an inactive session");
    let again = r.send(NotifMsg::Created(ev(5, "ai", "watch", "x", Some(9), true)));
    assert!(
        toasts(&again).is_empty(),
        "a repeated id (resubscription) produces no second toast"
    );
}

#[test]
fn snapshot_events_never_toast() {
    let mut r = rig(Vec::new());
    let items = vec![reply(1, 9), reply(2, 9)];
    let counts = counts_of(&items);
    let fx = r.send(NotifMsg::Snapshot { items, counts });
    assert!(
        toasts(&fx).is_empty(),
        "history at start and after reconnect is not news"
    );
}

#[test]
fn toast_without_known_session_name_shows_the_event_text() {
    let mut r = rig(Vec::new());
    let fx = r.send(NotifMsg::Created(reply(1, 9)));
    assert_eq!(toasts(&fx), vec![(9, "reply ready".to_string())]);
}

#[test]
fn toast_lives_four_seconds_on_the_shell_clock() {
    let mut r = rig(Vec::new());
    r.send(open(7));
    let fx = r.send(NotifMsg::Created(reply(1, 9)));
    let mut shell = ShellController::new(UiFile::default());
    let t0 = r.now;
    for event in published(&fx) {
        shell.update(ShellMsg::Bus(event), t0);
    }
    let shown = shell.view().toast.expect("toast shown");
    assert_eq!(shown.session_id, 9);
    assert_eq!(TOAST_DURATION, Duration::from_secs(4));
    shell.update(ShellMsg::Tick, t0 + Duration::from_millis(3999));
    assert!(
        shell.view().toast.is_some(),
        "at 3.999 s the toast is still visible"
    );
    shell.update(ShellMsg::Tick, t0 + Duration::from_secs(4));
    assert!(shell.view().toast.is_none(), "gone after 4 s");

    let fx = r.send(NotifMsg::Created(reply(2, 7)));
    for event in published(&fx) {
        shell.update(ShellMsg::Bus(event), t0);
    }
    assert!(
        shell.view().toast.is_none(),
        "no toast is created for the active session"
    );
}
