//! The feed against `fake-agentd` over a real socket: snapshot and deltas, counters, "read", resubscribe, D-Bus.

mod common;
mod notif_common;

use controllers::controllers::notifications::NotifMsg;
use controllers::notif_types::{EventKind, EventsFilter};
use controllers::{BusEvent, Controller, Tab};
use core_client::events::EventQuery;
use notif_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn events_snapshot_then_ws_delta() {
    let mut r = rig().await;
    let a = r.add("ai", "agent", "first", None);
    let b = r.add("msg", "wa", "second", None);
    r.until("both events arrived", |r| r.ids() == vec![b, a])
        .await;
    let c = r.add("task", "sched", "third", None);
    r.until("delta arrived", |r| r.ids().first() == Some(&c))
        .await;
    let page = r
        .http
        .list_events(&EventQuery::default())
        .await
        .expect("GET /events");
    let server: Vec<i64> = page["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e["id"].as_i64())
        .collect();
    assert_eq!(
        r.ids(),
        server,
        "snapshot plus a WS frame give the same list as GET /events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_response_has_counts() {
    let mut r = rig().await;
    for (kind, sub) in [
        ("ai", "agent"),
        ("ai", "agent"),
        ("ai", "watch"),
        ("msg", "wa"),
        ("task", "done"),
    ] {
        r.add(kind, sub, "x", None);
    }
    r.until("feed arrived", |r| r.ids().len() == 5).await;
    let page = r
        .http
        .list_events(&EventQuery {
            limit: Some(1),
            ..EventQuery::default()
        })
        .await
        .expect("GET /events");
    assert_eq!(
        page["items"].as_array().map(Vec::len),
        Some(1),
        "a page of one event"
    );
    assert_eq!(
        page["counts"]["total"], 5,
        "counts are over the selection, not the page count"
    );
    assert_eq!(page["counts"]["by_kind"]["ai"], r.db_unread("ai", None));
    assert_eq!(
        page["counts"]["by_sub"]["ai.agent"],
        r.db_unread("ai", Some("agent"))
    );
    assert_eq!(
        page["counts"]["by_sub"]["msg.wa"],
        r.db_unread("msg", Some("wa"))
    );
    let v = r.n.view();
    assert_eq!(v.total_unread, 5, "the controller sees the same counters");
    assert_eq!(
        v.kinds
            .iter()
            .find(|k| k.kind == EventKind::Ai)
            .map(|k| k.badge.count),
        Some(r.db_unread("ai", None))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_mark_all_read_route() {
    let mut r = rig().await;
    r.add("ai", "agent", "a", None);
    r.add("ai", "watch", "b", None);
    r.add("msg", "wa", "c", None);
    r.until("three events", |r| r.ids().len() == 3).await;
    let res = r
        .msg(NotifMsg::MarkRead(EventsFilter {
            kind: Some(EventKind::Ai),
            ..EventsFilter::default()
        }))
        .await;
    assert_eq!(res, vec![Ok(())], "POST /events/read accepted");
    assert_eq!(
        r.db_unread("ai", None),
        0,
        "unread matching the filter are marked in the core"
    );
    assert_eq!(r.db_unread("msg", None), 1, "the rest are not");
    r.until("the 'read' frame arrived", |r| {
        r.n.view().cards.iter().filter(|c| c.unread).count() == 1
    })
    .await;
    assert_eq!(r.n.view().total_unread, 1);
    let page = r
        .http
        .list_events(&EventQuery::default())
        .await
        .expect("GET /events");
    assert_eq!(
        page["counts"]["total"],
        r.n.view().total_unread,
        "controller and core counters are equal"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_tab_from_event_tab() {
    let mut r = rig().await;
    let s = r.first_session().await;
    let id = r.add("ai", "agent", "reply", Some(s));
    r.until("event arrived", |r| r.ids() == vec![id]).await;
    let item = r.n.view().cards.into_iter().next().expect("card");
    assert_eq!(item.session_id, Some(s));
    r.msg(NotifMsg::Open(id)).await;
    assert_eq!(
        r.bus.last(),
        Some(&BusEvent::OpenSession {
            session_id: s,
            tab: Tab::Chat
        }),
        "Event.tab = chat opens the chat"
    );

    // an old-version core: a frame without the `tab` field is read as chat
    let id2 = 5000;
    let frame = json!({"id": id2, "kind": "ai", "sub": "agent", "level": "info", "text": "no tab",
                       "session_id": s, "source": null, "unread": true, "ts": 1});
    r.d.emit("events", fake_agentd::FrameType::Event, frame);
    r.until("frame without tab arrived", |r| {
        r.ids().first() == Some(&id2)
    })
    .await;
    r.msg(NotifMsg::Open(id2)).await;
    assert_eq!(
        r.bus.last(),
        Some(&BusEvent::OpenSession {
            session_id: s,
            tab: Tab::Chat
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_resubscribe_after_reconnect() {
    let mut r = rig().await;
    let a = r.add("msg", "tg", "before the drop", None);
    r.until("first event", |r| r.ids() == vec![a]).await;
    let before = r.snapshots;
    r.d.drop_ws();
    let missed = r.add("msg", "mail", "while offline", None);
    r.d.restore();
    r.until("new snapshot after reconnecting", |r| {
        r.snapshots > before && r.ids().contains(&missed)
    })
    .await;
    assert_eq!(r.ids(), vec![missed, a], "the missed event caught up");
    assert_eq!(r.n.view().total_unread, 2);
    let late = r.add("ai", "local", "after reconnecting", None);
    r.until("delta after resubscribing", |r| {
        r.ids().first() == Some(&late)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn events_dbus_bridge_is_msg_sys() {
    let mut r = rig().await;
    let id =
        r.d.dbus_notification("Battery low")
            .expect("write")
            .expect("event");
    r.until("notification arrived", |r| r.ids() == vec![id])
        .await;
    let card = r.n.view().cards.into_iter().next().expect("card");
    assert_eq!((card.kind.as_str(), card.sub.as_str()), ("msg", "sys"));
    assert_eq!(card.text, "Battery low");
    assert_eq!(card.session_id, None);
    let row = r
        .db_events()
        .into_iter()
        .find(|e| e.0 == id)
        .expect("in the DB");
    assert_eq!((row.1.as_str(), row.2.as_str()), ("msg", "sys"));
}

#[tokio::test(flavor = "multi_thread")]
async fn events_survive_session_delete() {
    let mut r = rig().await;
    let s = r.first_session().await;
    let id = r.add("ai", "agent", "reply", Some(s));
    r.until("event arrived", |r| r.ids() == vec![id]).await;
    assert_eq!(r.n.view().cards[0].session_id, Some(s));
    r.http.delete_session(s).await.expect("session deletion");
    r.until("session deleted in the feed", |r| {
        r.n.view()
            .cards
            .first()
            .is_some_and(|c| c.session_id.is_none())
    })
    .await;
    assert_eq!(r.ids(), vec![id], "the event stayed in the feed");
    let row = r
        .db_events()
        .into_iter()
        .find(|e| e.0 == id)
        .expect("in the DB");
    assert_eq!(
        row.3, None,
        "the core nulled session_id, the event was not erased"
    );
    assert_eq!(r.n.view().total_unread, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_registry_extends_filters_without_client_change() {
    let mut r = rig().await;
    r.d.register_subtype("msg", "signal", "Signal", Some("signal-symbolic"));
    r.load_subtypes().await;
    r.msg(NotifMsg::SelectKind(EventKind::Msg)).await;
    let chip =
        r.n.view()
            .kinds
            .into_iter()
            .find(|k| k.kind == EventKind::Msg)
            .expect("msg kind");
    let subs: Vec<&str> = chip.subs.iter().map(|s| s.sub.as_str()).collect();
    assert_eq!(
        subs,
        vec!["sys", "wa", "tg", "mail", "signal"],
        "subtypes came from the core"
    );
    let id = r.add("msg", "signal", "new provider", None);
    r.until("event of the new subtype", |r| r.ids() == vec![id])
        .await;
    r.msg(NotifMsg::SelectSub(EventKind::Msg, "signal".into()))
        .await;
    assert_eq!(r.ids(), vec![id]);
    assert_eq!(r.n.view().cards[0].sub_label, "Signal");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_forged_and_hostile_frames_are_handled() {
    let mut r = rig().await;
    r.d.emit(
        "events",
        fake_agentd::FrameType::Event,
        json!({"id": 1, "kind": "bogus", "sub": "x", "text": "t", "unread": true}),
    );
    r.d.emit(
        "events",
        fake_agentd::FrameType::Event,
        json!("not an object"),
    );
    r.d.emit(
        "events",
        fake_agentd::FrameType::Delta,
        json!({"read": "not an array"}),
    );
    r.d.emit(
        "events",
        fake_agentd::FrameType::Delta,
        json!({"unknown": 1}),
    );
    let evil = "<img src=x onerror=alert(1)> & \" ' javascript:alert(1)";
    let id = r.add("ai", "agent", evil, None);
    r.until("the real one arrived after the garbage frames", |r| {
        r.ids() == vec![id]
    })
    .await;
    assert_eq!(r.n.view().cards[0].text, evil, "the text arrived unchanged");
    assert_eq!(
        r.n.view().total_unread,
        1,
        "garbage did not touch the counters"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_read_commands_fail_with_read_scope_token() {
    let mut r = rig().await;
    let id = r.add("ai", "agent", "x", None);
    r.until("event arrived", |r| r.ids() == vec![id]).await;
    let token = crate::common::issue(&r.d, fake_agentd::Scope::Read, fake_agentd::Kind::External);
    r.http = crate::common::http(&r.d, Some(&token));
    let res = r.msg(NotifMsg::MarkRead(EventsFilter::default())).await;
    assert_eq!(res.len(), 1);
    let err = res[0].clone().expect_err("a read token cannot mark");
    assert!(
        !err.contains(&token),
        "the token does not appear in the error text"
    );
    assert_eq!(r.db_unread("ai", None), 1, "the core changed nothing");
}
