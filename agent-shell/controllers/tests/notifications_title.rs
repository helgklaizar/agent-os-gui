//! Window title `(N) Agent OS`, empty feed states, strings from `en.ftl`.

mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::i18n::{EN_FTL, tr};
use controllers::notif_types::EventKind;
use notif_common::*;

#[test]
fn notifications_window_title_unread_count() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "a", None, true),
        ev(2, "msg", "wa", "b", None, true),
        ev(3, "task", "done", "c", None, true),
        ev(4, "task", "done", "d", None, false),
    ]);
    assert_eq!(r.view().title, "(3) Agent OS");
    r.send(NotifMsg::Read(vec![1]));
    assert_eq!(
        r.view().title,
        "(2) Agent OS",
        "read - the number went down"
    );
    r.send(NotifMsg::Created(ev(5, "msg", "tg", "e", None, true)));
    assert_eq!(r.view().title, "(3) Agent OS");
    r.send(NotifMsg::Read(vec![2, 3, 5]));
    assert_eq!(r.view().title, "Agent OS", "no prefix at zero");
}

#[test]
fn window_title_shows_exact_count_beyond_badge_cap() {
    let items: Vec<_> = (1..=120)
        .map(|i| ev(i, "msg", "sys", "x", None, true))
        .collect();
    let r = rig(items);
    assert_eq!(
        r.view().title,
        "(120) Agent OS",
        "the exact number in the title"
    );
    assert_eq!(
        r.view().all.badge.text.as_deref(),
        Some("99+"),
        "the badge shows \"99+\""
    );
}

#[test]
fn window_title_is_plain_before_any_snapshot() {
    let r = rig(Vec::new());
    assert_eq!(r.view().title, "Agent OS");
}

#[test]
fn notifications_empty_states() {
    let mut r = rig(Vec::new());
    assert_eq!(
        r.view().empty.as_deref(),
        Some("Nothing here yet…"),
        "empty feed"
    );
    assert!(r.view().cards.is_empty());
    r.send(NotifMsg::SelectKind(EventKind::Ai));
    assert_eq!(
        r.view().empty.as_deref(),
        Some("Nothing here yet…"),
        "no events at all - the filter is not the cause"
    );

    r.send(NotifMsg::Created(ev(1, "msg", "wa", "x", None, true)));
    assert_eq!(
        r.view().empty.as_deref(),
        Some("Nothing matches this filter"),
        "filter without results"
    );
    assert!(r.view().cards.is_empty());
    r.send(NotifMsg::SelectAll);
    assert_eq!(r.view().empty, None, "there are cards - no hint");
    assert_eq!(r.card_ids(), vec![1]);
}

#[test]
fn i18n_notifications_strings_from_ftl() {
    for (key, text) in [
        ("notifications-empty", "Nothing here yet…"),
        ("notifications-filter-empty", "Nothing matches this filter"),
        ("notifications-kind-ai", "AI"),
        ("notifications-kind-task", "Tasks"),
        ("notifications-kind-msg", "Messages"),
        ("notifications-all", "All"),
    ] {
        assert!(
            EN_FTL.contains(&format!("{key} = {text}")),
            "{key} in en.ftl"
        );
        assert_eq!(tr(key), text);
    }
    let mut r = rig(Vec::new());
    assert_eq!(r.view().empty, Some(tr("notifications-empty")));
    assert_eq!(r.kind(EventKind::Ai).label, tr("notifications-kind-ai"));
    assert_eq!(r.kind(EventKind::Task).label, tr("notifications-kind-task"));
    assert_eq!(r.kind(EventKind::Msg).label, tr("notifications-kind-msg"));
    assert_eq!(r.view().all.label, tr("notifications-all"));
    r.send(NotifMsg::Created(ev(1, "msg", "wa", "x", None, true)));
    r.send(NotifMsg::SelectKind(EventKind::Ai));
    assert_eq!(r.view().empty, Some(tr("notifications-filter-empty")));
}

#[test]
fn badge_a11y_name_reads_unread_count() {
    let r = rig(vec![
        ev(1, "ai", "agent", "a", None, true),
        ev(2, "ai", "agent", "b", None, true),
    ]);
    assert_eq!(r.view().all.badge.a11y_name, "All: 2 unread");
    assert_eq!(r.kind(EventKind::Ai).badge.a11y_name, "AI: 2 unread");
    assert_eq!(r.kind(EventKind::Msg).badge.text, None, "zero - no badge");
}
