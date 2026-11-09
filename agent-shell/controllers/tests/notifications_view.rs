//! S5-05: feed cards: time, subtype, session name ("parent › thread" for a thread), the "new" label, the colour stripe,
//! the button; an unknown subtype and hostile text do not break the card.

mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::notif_view::{Accent, CardAction};
use controllers::testkit::evil_strings;
use controllers::tree_types::{SessionItem, TreeFrame};
use notif_common::*;

fn session(id: i64, parent: Option<i64>, name: &str) -> SessionItem {
    SessionItem {
        id,
        parent_id: parent,
        name: name.to_string(),
        ..SessionItem::default()
    }
}

fn with_sessions(r: &mut Rig) {
    r.send(NotifMsg::Sessions(vec![
        session(7, None, "Release"),
        session(8, Some(7), "Review"),
    ]));
}

#[test]
fn notifications_card_fields() {
    let mut r = rig(vec![
        ev(1, "ai", "agent", "reply ready", Some(7), true),
        ev(2, "task", "done", "task completed", None, false),
        ev(3, "msg", "wa", "Anna: hello", None, true),
    ]);
    with_sessions(&mut r);
    let c = r.card(1).expect("card 1");
    assert_eq!((c.kind.as_str(), c.sub.as_str()), ("ai", "agent"));
    assert_eq!(c.sub_label, "Agent", "subtype label from the registry");
    assert_eq!(
        c.icon.as_deref(),
        Some("agent-icon"),
        "icon from the registry"
    );
    assert_eq!(c.text, "reply ready");
    assert_eq!(c.session_id, Some(7));
    assert_eq!(c.session_label.as_deref(), Some("Release"));
    assert_eq!(c.time, "22:14", "HH:MM in UTC at zero offset");
    assert_eq!(c.accent, Accent::Ai);
    assert_eq!(c.level, "info");
    assert_eq!(c.new_label.as_deref(), Some("new"), "unread is marked");
    assert!(c.unread);
    assert_eq!(
        (c.action, c.action_label.as_str()),
        (CardAction::OpenChat, "Open chat")
    );

    let done = r.card(2).expect("card 2");
    assert_eq!(done.accent, Accent::Task);
    assert_eq!(done.new_label, None, "a read event has no 'new' label");
    assert_eq!(done.session_label, None);
    assert_eq!(
        (done.action, done.action_label.as_str()),
        (CardAction::Ok, "OK"),
        "without a session: 'OK'"
    );

    assert_eq!(r.card(3).map(|c| c.accent), Some(Accent::Msg));
}

#[test]
fn notifications_card_time_follows_user_zone() {
    let mut r = rig(vec![ev(1, "msg", "sys", "x", None, true)]);
    r.send(NotifMsg::UtcOffset(7200));
    assert_eq!(
        r.card(1).map(|c| c.time),
        Some("00:14".to_string()),
        "UTC+2 crosses midnight"
    );
    r.send(NotifMsg::UtcOffset(-3600));
    assert_eq!(r.card(1).map(|c| c.time), Some("21:14".to_string()));
}

#[test]
fn notifications_thread_card_shows_parent_path() {
    let mut r = rig(vec![
        ev(1, "ai", "watch", "in a thread", Some(8), true),
        ev(2, "ai", "agent", "in a session", Some(7), true),
    ]);
    with_sessions(&mut r);
    assert_eq!(
        r.card(1).and_then(|c| c.session_label).as_deref(),
        Some("Release › Review")
    );
    assert_eq!(
        r.card(2).and_then(|c| c.session_label).as_deref(),
        Some("Release")
    );
    r.send(NotifMsg::Tree(TreeFrame::SessionUpdated(session(
        8,
        Some(7),
        "Hotfix",
    ))));
    assert_eq!(
        r.card(1).and_then(|c| c.session_label).as_deref(),
        Some("Release › Hotfix"),
        "the name follows a rename"
    );
}

#[test]
fn notifications_unknown_subtype_generic_card() {
    let mut r = rig(vec![ev(
        1,
        "msg",
        "signal",
        "from a new provider",
        None,
        true,
    )]);
    let c = r.card(1).expect("card without a panic");
    assert_eq!(
        c.sub_label, "signal",
        "no label in the registry: the sub itself"
    );
    assert_eq!(c.icon, None);
    assert_eq!(c.text, "from a new provider");
    assert_eq!(c.accent, Accent::Msg);
    r.send(NotifMsg::Subtypes({
        let mut reg = registry();
        reg.push(controllers::notif_types::SubtypeItem {
            kind: "msg".into(),
            sub: "signal".into(),
            label: "Signal".into(),
            icon: None,
            enabled: true,
        });
        reg
    }));
    assert_eq!(
        r.card(1).map(|c| c.sub_label),
        Some("Signal".to_string()),
        "label from the registry once it has grown"
    );
    let empty_label = {
        let mut reg = registry();
        reg.push(controllers::notif_types::SubtypeItem {
            kind: "msg".into(),
            sub: "signal".into(),
            label: String::new(),
            icon: None,
            enabled: true,
        });
        reg
    };
    r.send(NotifMsg::Subtypes(empty_label));
    assert_eq!(
        r.card(1).map(|c| c.sub_label),
        Some("signal".to_string()),
        "an empty label also falls back to sub"
    );
}

#[test]
fn notifications_text_is_escaped() {
    for (i, evil) in evil_strings().into_iter().enumerate() {
        let id = i as i64 + 1;
        let mut r = rig(vec![ev(id, "msg", &evil, &evil, Some(7), true)]);
        r.send(NotifMsg::Sessions(vec![session(7, None, &evil)]));
        let c = r.card(id).expect("card");
        assert_eq!(
            c.text, evil,
            "event text unchanged (the view shows it literally)"
        );
        assert_eq!(c.sub_label, evil);
        assert_eq!(c.session_label.as_deref(), Some(evil.as_str()));
    }
}

#[test]
fn notifications_deleted_session_makes_card_system() {
    let mut r = rig(vec![ev(1, "ai", "agent", "reply", Some(7), true)]);
    with_sessions(&mut r);
    r.send(NotifMsg::Tree(TreeFrame::SessionDeleted(7)));
    let c = r.card(1).expect("the event stayed in the feed");
    assert_eq!(c.session_id, None, "session_id cleared, as in the core");
    assert_eq!(c.session_label, None);
    assert_eq!(c.action, CardAction::Ok);
}
