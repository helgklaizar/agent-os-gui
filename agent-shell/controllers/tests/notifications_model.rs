//! Feed model: snapshot and deltas, the `All → kind → subtype` filter, unread counters, the registry, "read".

mod notif_common;

use controllers::CoreCommand;
use controllers::controllers::notifications::NotifMsg;
use controllers::notif_types::{EventKind, EventsFilter, SubtypeItem};
use controllers::notif_view::Selection;
use notif_common::*;

fn mixed() -> Vec<controllers::notif_types::EventItem> {
    vec![
        ev(1, "ai", "agent", "a1", Some(7), true),
        ev(2, "ai", "watch", "a2", None, true),
        ev(3, "task", "done", "t1", Some(7), true),
        ev(4, "msg", "wa", "m1", None, true),
        ev(5, "msg", "wa", "m2", None, false),
    ]
}

#[test]
fn notifications_filter_all_kind_sub_toggle() {
    let mut r = rig(mixed());
    assert_eq!(r.view().selection, Selection::All);
    assert_eq!(r.card_ids(), vec![5, 4, 3, 2, 1], "all, newest first");

    r.send(NotifMsg::SelectKind(EventKind::Ai));
    assert_eq!(r.view().selection, Selection::Kind(EventKind::Ai));
    assert_eq!(r.card_ids(), vec![2, 1]);
    assert!(r.kind(EventKind::Ai).active);
    assert!(!r.kind(EventKind::Task).active);

    r.send(NotifMsg::SelectSub(EventKind::Ai, "watch".into()));
    assert_eq!(
        r.view().selection,
        Selection::Sub(EventKind::Ai, "watch".into())
    );
    assert_eq!(r.card_ids(), vec![2]);
    assert!(r.sub(EventKind::Ai, "watch").is_some_and(|s| s.active));

    r.send(NotifMsg::SelectSub(EventKind::Ai, "watch".into()));
    assert_eq!(
        r.view().selection,
        Selection::Kind(EventKind::Ai),
        "clicking the active subtype again clears it"
    );
    assert_eq!(r.card_ids(), vec![2, 1]);

    r.send(NotifMsg::SelectKind(EventKind::Ai));
    assert_eq!(
        r.view().selection,
        Selection::All,
        "clicking the active kind again clears the kind"
    );

    r.send(NotifMsg::SelectSub(EventKind::Msg, "wa".into()));
    r.send(NotifMsg::SelectKind(EventKind::Task));
    assert_eq!(
        r.view().selection,
        Selection::Kind(EventKind::Task),
        "another kind replaces the subtype"
    );
    r.send(NotifMsg::SelectAll);
    assert_eq!(r.view().selection, Selection::All);
    assert!(r.view().all.active);
}

#[test]
fn notifications_counters_count_only_unread() {
    let mut r = rig(mixed());
    let v = r.view();
    assert_eq!(v.total_unread, 4, "the read m2 is not counted");
    assert_eq!(v.all.badge.count, 4);
    assert_eq!(r.kind(EventKind::Ai).badge.count, 2);
    assert_eq!(r.kind(EventKind::Task).badge.count, 1);
    assert_eq!(
        r.kind(EventKind::Msg).badge.count,
        1,
        "msg has two events, one unread"
    );
    r.send(NotifMsg::SelectKind(EventKind::Msg));
    assert_eq!(r.sub(EventKind::Msg, "wa").map(|s| s.badge.count), Some(1));
    assert_eq!(r.sub(EventKind::Msg, "tg").map(|s| s.badge.count), Some(0));
    r.send(NotifMsg::Read(vec![4]));
    assert_eq!(r.kind(EventKind::Msg).badge.count, 0);
    assert_eq!(r.view().total_unread, 3);
    assert_eq!(r.card_ids().len(), 2, "read events stay in the feed");
}

#[test]
fn notifications_every_subtype_reaches_its_filter() {
    for (kind, sub, _, _) in REGISTRY {
        let mut r = rig(Vec::new());
        r.send(NotifMsg::Created(ev(1, kind, sub, "new", None, true)));
        let k = EventKind::parse(kind).expect("kind from the registry");
        assert_eq!(r.view().all.badge.count, 1, "{kind}/{sub}: total counter");
        assert_eq!(r.kind(k).badge.count, 1, "{kind}/{sub}: kind counter");
        r.send(NotifMsg::SelectSub(k, sub.to_string()));
        assert_eq!(
            r.card_ids(),
            vec![1],
            "{kind}/{sub}: event in its own filter"
        );
        assert_eq!(
            r.sub(k, sub).map(|s| s.badge.count),
            Some(1),
            "{kind}/{sub}: subtype counter grows"
        );
        for (other_kind, other, _, _) in REGISTRY {
            if (other_kind, other) != (kind, sub) && other_kind == kind {
                assert_eq!(
                    r.sub(k, other).map(|s| s.badge.count),
                    Some(0),
                    "foreign subfilter {other}"
                );
            }
        }
    }
}

#[test]
fn notifications_registry_drives_filters() {
    let mut r = rig(Vec::new());
    r.send(NotifMsg::SelectKind(EventKind::Msg));
    let subs = |r: &Rig| -> Vec<String> {
        r.kind(EventKind::Msg)
            .subs
            .into_iter()
            .map(|s| s.sub)
            .collect()
    };
    assert_eq!(
        subs(&r),
        vec!["sys", "wa", "tg", "mail"],
        "subfilters from the registry, in its order"
    );
    let ai_subs: Vec<String> = r
        .kind(EventKind::Ai)
        .subs
        .into_iter()
        .map(|s| s.sub)
        .collect();
    assert!(
        ai_subs.is_empty(),
        "subfilters are shown for the selected kind"
    );

    let mut with_signal = registry();
    with_signal.push(SubtypeItem {
        kind: "msg".into(),
        sub: "signal".into(),
        label: "Signal".into(),
        icon: Some("signal-icon".into()),
        enabled: true,
    });
    r.send(NotifMsg::Subtypes(with_signal));
    assert_eq!(
        subs(&r),
        vec!["sys", "wa", "tg", "mail", "signal"],
        "a new subtype is a new filter without code changes"
    );
    let chip = r.sub(EventKind::Msg, "signal");
    assert_eq!(chip.as_ref().map(|c| c.label.as_str()), Some("Signal"));
    assert_eq!(chip.and_then(|c| c.icon), Some("signal-icon".to_string()));

    let mut shorter = registry();
    shorter.retain(|s| s.sub != "tg");
    shorter
        .iter_mut()
        .filter(|s| s.sub == "mail")
        .for_each(|s| s.enabled = false);
    r.send(NotifMsg::Subtypes(shorter));
    assert_eq!(
        subs(&r),
        vec!["sys", "wa"],
        "removed and disabled subtypes disappear"
    );
}

#[test]
fn notifications_mark_all_read_by_filter() {
    let mut r = rig(mixed());
    let fx = r.send(NotifMsg::MarkRead(EventsFilter {
        kind: Some(EventKind::Ai),
        ..EventsFilter::default()
    }));
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::MarkEventsRead(EventsFilter {
            kind: Some(EventKind::Ai),
            ..EventsFilter::default()
        })]
    );
    assert_eq!(
        r.kind(EventKind::Ai).badge.count,
        0,
        "the filter counters are zero"
    );
    assert_eq!(
        r.kind(EventKind::Task).badge.count,
        1,
        "the others do not change"
    );
    assert_eq!(r.kind(EventKind::Msg).badge.count, 1);
    assert_eq!(r.view().total_unread, 2);

    let fx = r.send(NotifMsg::MarkRead(EventsFilter {
        kind: Some(EventKind::Msg),
        sub: Some("wa".into()),
        ..EventsFilter::default()
    }));
    assert_eq!(effects_of_commands(&fx).len(), 1);
    assert_eq!(r.kind(EventKind::Msg).badge.count, 0);
    assert_eq!(r.kind(EventKind::Task).badge.count, 1);

    r.send(NotifMsg::MarkRead(EventsFilter {
        session_id: Some(7),
        ..EventsFilter::default()
    }));
    assert_eq!(r.view().total_unread, 0, "event 3 of session 7 is marked");
}

#[test]
fn notifications_snapshot_replaces_state_and_created_is_idempotent() {
    let mut r = rig(mixed());
    r.send(NotifMsg::Created(ev(4, "msg", "wa", "m1", None, true)));
    assert_eq!(
        r.view().total_unread,
        4,
        "a repeated id (after resubscribing) does not double the counter"
    );
    assert_eq!(r.card_ids().len(), 5);
    r.send(NotifMsg::Created(ev(6, "ai", "local", "new", None, true)));
    assert_eq!(r.card_ids()[0], 6, "the new event comes first");
    assert_eq!(r.view().total_unread, 5);
    let fresh = vec![ev(9, "task", "sched", "after reconnect", None, true)];
    let counts = counts_of(&fresh);
    r.send(NotifMsg::Snapshot {
        items: fresh,
        counts,
    });
    assert_eq!(r.card_ids(), vec![9], "the snapshot replaces the feed");
    assert_eq!(r.view().total_unread, 1);
}

#[test]
fn notifications_read_delta_adjusts_once_and_resyncs_on_unknown_id() {
    let mut r = rig(mixed());
    let fx = r.send(NotifMsg::Read(vec![1]));
    assert!(
        effects_of_commands(&fx).is_empty(),
        "known event: no resync"
    );
    assert_eq!(r.view().total_unread, 3);
    r.send(NotifMsg::Read(vec![1]));
    assert_eq!(
        r.view().total_unread,
        3,
        "a repeated 'read' does not decrease twice"
    );
    let fx = r.send(NotifMsg::Read(vec![999]));
    assert_eq!(
        effects_of_commands(&fx),
        vec![CoreCommand::ResyncEvents],
        "read something that is not in the feed: the counters may have diverged"
    );
}
