//! Following a card and the agent reply against `fake-agentd`.

mod common;
mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::controllers::session_tree::TreeMsg;
use controllers::{BusEvent, Controller, Tab};
use notif_common::*;
use std::time::Instant;

#[tokio::test(flavor = "multi_thread")]
async fn events_open_chat_switches_session_and_tab() {
    let mut r = rig().await;
    let s = r.first_session().await;
    let other = r
        .http
        .create_session("Another", None, None)
        .await
        .expect("session")["id"]
        .as_i64()
        .unwrap_or(-1);
    let a = r.add("ai", "agent", "reply 1", Some(s));
    let b = r.add("task", "done", "task", Some(s));
    let c = r.add("ai", "agent", "in another", Some(other));
    let sys = r.add("msg", "sys", "system", None);
    r.until("four events", |r| r.ids().len() == 4).await;
    let res = r.msg(NotifMsg::Open(a)).await;
    assert!(res.iter().all(Result::is_ok), "{res:?}");
    assert_eq!(
        r.bus.last(),
        Some(&BusEvent::OpenSession {
            session_id: s,
            tab: Tab::Chat
        }),
        "the right session and tab"
    );
    assert_eq!(
        r.tree.view().selected,
        Some(s),
        "the tree selected the session"
    );
    let unread: Vec<(i64, bool)> = r.db_events().into_iter().map(|e| (e.0, e.4)).collect();
    assert_eq!(
        unread,
        vec![(a, false), (b, false), (c, true), (sys, true)],
        "this session's unread are marked in the core"
    );
    r.until("'read' frames arrived", |r| r.n.view().total_unread == 2)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn events_ok_without_session_marks_only_that_event() {
    let mut r = rig().await;
    let a = r.add("msg", "sys", "one", None);
    let b = r.add("msg", "sys", "another", None);
    r.until("two events", |r| r.ids().len() == 2).await;
    let res = r.msg(NotifMsg::Open(a)).await;
    assert_eq!(res, vec![Ok(())]);
    assert!(
        !r.bus
            .iter()
            .any(|e| matches!(e, BusEvent::OpenSession { .. })),
        "no such session: nothing opened"
    );
    let unread: Vec<(i64, bool)> = r.db_events().into_iter().map(|e| (e.0, e.4)).collect();
    assert_eq!(unread, vec![(a, false), (b, true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_agent_reply_in_inactive_session_gives_toast_badge_and_event() {
    let mut r = rig().await;
    let active = r.first_session().await;
    let quiet = r
        .http
        .create_session("Quiet", None, None)
        .await
        .expect("session")["id"]
        .as_i64()
        .unwrap_or(-1);
    r.until("both sessions in the tree", |r| {
        r.tree.view().rows.len() >= 2
    })
    .await;
    r.run(vec![controllers::Effect::Publish(BusEvent::OpenSession {
        session_id: active,
        tab: Tab::Chat,
    })])
    .await;
    r.bus.clear();
    let sent = r
        .http
        .send_message(quiet, "00000000-0000-4000-8000-000000000001", "hello", &[])
        .await;
    assert!(sent.is_ok(), "{sent:?}");
    r.until("agent reply: an event in the feed", |r| {
        r.n.view().cards.iter().any(|c| c.session_id == Some(quiet))
    })
    .await;
    let toasts: Vec<&BusEvent> = r
        .bus
        .iter()
        .filter(|e| matches!(e, BusEvent::ShowToast { .. }))
        .collect();
    assert_eq!(
        toasts.len(),
        1,
        "toast for a reply in an inactive session: {:?}",
        r.bus
    );
    assert!(matches!(toasts[0], BusEvent::ShowToast { session_id, .. } if *session_id == quiet));
    r.until("session badge", |r| {
        r.tree
            .view()
            .rows
            .iter()
            .any(|row| row.badge.is_some() && row.name == "Quiet")
    })
    .await;
    r.until("event waits to be read", |r| r.n.view().total_unread == 1)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn events_agent_reply_in_active_session_has_no_toast() {
    let mut r = rig().await;
    let s = r.first_session().await;
    r.run(vec![controllers::Effect::Publish(BusEvent::OpenSession {
        session_id: s,
        tab: Tab::Chat,
    })])
    .await;
    r.tree.update(TreeMsg::Opened(s), Instant::now());
    r.bus.clear();
    let sent = r
        .http
        .send_message(s, "00000000-0000-4000-8000-000000000002", "hello", &[])
        .await;
    assert!(sent.is_ok(), "{sent:?}");
    r.until("the reply event arrived and was read", |r| {
        r.n.view()
            .cards
            .iter()
            .any(|c| c.session_id == Some(s) && !c.unread)
    })
    .await;
    assert!(
        !r.bus
            .iter()
            .any(|e| matches!(e, BusEvent::ShowToast { .. })),
        "no toast for the active session"
    );
    assert_eq!(
        r.db_unread("ai", Some("agent")),
        0,
        "the event is read in the core too"
    );
}
