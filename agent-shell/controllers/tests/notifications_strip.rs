//! S5-06: the collapsed strip shows kind icons with badges, a click expands the column with that filter, including in a narrow window.

mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::controllers::shell::{ShellController, ShellMsg};
use controllers::notif_types::EventKind;
use controllers::notif_view::Selection;
use controllers::prefs::UiFile;
use controllers::{BusEvent, Column, Controller, Effect, EventBus};
use notif_common::*;
use std::cell::RefCell;
use std::rc::Rc;

fn items() -> Vec<controllers::notif_types::EventItem> {
    vec![
        ev(1, "ai", "agent", "a", None, true),
        ev(2, "ai", "watch", "b", None, true),
        ev(3, "msg", "wa", "c", None, true),
        ev(4, "task", "done", "d", None, false),
    ]
}

#[test]
fn strip_is_empty_when_expanded_and_has_three_kinds_when_collapsed() {
    let mut r = rig(items());
    assert!(r.view().strip.is_empty(), "an expanded column has no strip");
    assert!(!r.view().collapsed);
    r.send(NotifMsg::Collapsed(true));
    let v = r.view();
    assert!(v.collapsed);
    let kinds: Vec<EventKind> = v.strip.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, vec![EventKind::Ai, EventKind::Task, EventKind::Msg]);
    let badges: Vec<Option<String>> = v.strip.iter().map(|s| s.badge.text.clone()).collect();
    assert_eq!(
        badges,
        vec![Some("2".into()), None, Some("1".into())],
        "a badge on a kind with unread events"
    );
    for s in &v.strip {
        assert!(!s.icon.is_empty() && !s.label.is_empty());
        assert!(
            s.badge.a11y_name.contains("unread"),
            "{}",
            s.badge.a11y_name
        );
    }
    r.send(NotifMsg::Collapsed(false));
    assert!(r.view().strip.is_empty());
}

#[test]
fn notifications_collapsed_strip_expands_with_filter() {
    let mut r = rig(items());
    r.send(NotifMsg::Collapsed(true));
    let fx = r.send(NotifMsg::StripClick(EventKind::Msg));
    assert_eq!(
        published(&fx),
        vec![BusEvent::ExpandColumn(Column::Notifications)],
        "clicking the icon requests expanding the column"
    );
    assert_eq!(
        r.view().selection,
        Selection::Kind(EventKind::Msg),
        "the column expands with that filter"
    );
    assert_eq!(r.card_ids(), vec![3]);
}

#[test]
fn strip_click_on_active_kind_keeps_the_filter() {
    let mut r = rig(items());
    r.send(NotifMsg::StripClick(EventKind::Ai));
    r.send(NotifMsg::StripClick(EventKind::Ai));
    assert_eq!(
        r.view().selection,
        Selection::Kind(EventKind::Ai),
        "on the strip a second click does not clear the filter"
    );
}

/// Narrow window: 1024 px is below the 1072 threshold, the notifications column is collapsed automatically.
#[test]
fn collapsed_strip_expands_on_narrow_window() {
    let mut prefs = UiFile::default();
    prefs.window.w = 1024;
    let mut shell = ShellController::new(prefs);
    assert!(
        shell.view().layout.notifications_collapsed,
        "at 1024 px the column collapses by itself"
    );

    let bus = EventBus::new();
    let seen: Rc<RefCell<Vec<BusEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    bus.subscribe(move |e| sink.borrow_mut().push(e.clone()));

    let mut r = rig(items());
    r.send(NotifMsg::Collapsed(
        shell.view().layout.notifications_collapsed,
    ));
    let now = r.now;
    for fx in [r.send(NotifMsg::StripClick(EventKind::Ai))] {
        for e in fx {
            if let Effect::Publish(b) = e {
                bus.publish(b);
            }
        }
    }
    let events: Vec<BusEvent> = seen.borrow().clone();
    let mut persisted = false;
    for e in events {
        for fx in shell.update(ShellMsg::Bus(e), now) {
            persisted |= matches!(fx, Effect::Persist(_));
        }
    }
    let layout = shell.view().layout;
    assert!(
        !layout.notifications_collapsed,
        "manual expansion works in a narrow window"
    );
    assert_eq!(layout.notifications_w, 300);
    assert!(persisted, "the choice is persisted in ui.json");
    shell.update(ShellMsg::WindowResized { w: 1024, h: 700 }, now);
    assert!(
        !shell.view().layout.notifications_collapsed,
        "the manual state survives a layout recompute"
    );
    r.send(NotifMsg::Collapsed(layout.notifications_collapsed));
    assert!(
        r.view().strip.is_empty(),
        "the strip disappeared along with the collapsed view"
    );
}
