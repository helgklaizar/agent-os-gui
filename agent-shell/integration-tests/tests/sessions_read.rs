//! `POST /sessions/{id}/read`, a vanished session and a WS drop against `fake-agentd`.

mod common;
mod sessions_common;

use std::time::{Duration, Instant};

use controllers::controllers::session_tree::TreeMsg;
use controllers::tree_types::NodeId;
use controllers::{Controller, CoreCommand};
use core_client::ConnState;
use sessions_common::*;
use ui::tree_exec::execute;
use ui::tree_feed::route;

#[tokio::test(flavor = "multi_thread")]
async fn session_read_route_resets_unread() {
    let mut r = rig().await;
    let Some(NodeId::Session(release)) = r.name_node("Release 1.2") else {
        panic!("no 'Release 1.2'")
    };
    let badge = |r: &Rig| {
        r.tree
            .view()
            .rows
            .iter()
            .find(|x| x.node == s(release))
            .and_then(|x| x.badge.clone())
    };
    assert_eq!(badge(&r).as_deref(), Some("3"), "fixture: unread = 3");
    r.d.add_unread(release, 2).expect("increment");
    r.pump("badge grew", |t| {
        t.view()
            .rows
            .iter()
            .any(|x| x.node == s(release) && x.badge.as_deref() == Some("5"))
    })
    .await;
    let results = r.tree_msg(TreeMsg::Select(release)).await;
    assert_eq!(results, vec![Ok(())], "the reset command accepted");
    assert_eq!(badge(&r), None, "the badge disappeared at once");
    let unread: i64 =
        r.d.db()
            .reader()
            .expect("reader")
            .query_row(
                "SELECT unread FROM sessions WHERE id = ?1",
                [release],
                |x| x.get(0),
            )
            .expect("row");
    assert_eq!(unread, 0, "reset in the DB");
    r.d.add_unread(release, 1).expect("increment");
    r.pump("unread frame for the open session", |t| {
        t.view().selected == Some(release)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    while let Ok(ev) = r.rx.try_recv() {
        let fx = route(&mut r.tree, &ev, Instant::now()).effects;
        r.run(fx).await;
    }
    assert_eq!(
        badge(&r),
        None,
        "in the open session a new message is read at once"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_route_for_missing_session_is_an_error_not_a_crash() {
    let r = rig().await;
    let res = execute(&r.http, &CoreCommand::MarkRead { id: 99_999 }).await;
    let Some(Err(text)) = res else {
        panic!("expected an error, got {res:?}")
    };
    assert!(!text.is_empty());
    assert!(
        !text.contains(r.d.admin_token()),
        "the token does not appear in the error text"
    );
    let res = execute(&r.http, &CoreCommand::DeleteSession { id: 99_999 }).await;
    assert!(matches!(res, Some(Err(_))));
    assert!(
        execute(&r.http, &CoreCommand::FetchAbout).await.is_none(),
        "a foreign command is not executed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_survives_ws_drop() {
    let mut r = rig().await;
    let before = r.tree.view().rows.len();
    r.d.drop_ws();
    r.pump_state(ConnState::Reconnecting).await;
    assert!(r.bar.view().bar.offline, "'No connection' banner");
    assert_eq!(r.tree.view().rows.len(), before, "the tree stays on screen");
    let created = r.http.create_session("While offline", None, None).await;
    let id = created.as_ref().map(session_id).unwrap_or(-1);
    assert!(id > 0, "HTTP works with a dropped WS: {created:?}");
    r.d.restore();
    r.until("sync after reconnecting and removing the banner", |r| {
        r.tree.view().rows.iter().any(|x| x.node == s(id)) && !r.bar.view().bar.offline
    })
    .await;
}
