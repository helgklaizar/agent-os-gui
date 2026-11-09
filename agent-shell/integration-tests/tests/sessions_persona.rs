//! Persona, inheritance by a thread, moving and renaming against `fake-agentd`.

mod common;
mod sessions_common;

use std::time::Instant;

use controllers::Controller;
use controllers::controllers::dialog::DialogMsg;
use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::tree_types::{NodeId, parse_personas};
use serde_json::{Value, json};
use sessions_common::*;

#[tokio::test(flavor = "multi_thread")]
async fn persona_change_propagates() {
    let mut r = rig().await;
    let personas = parse_personas(&Value::Array(
        r.http.list_personas().await.unwrap_or_default(),
    ));
    let home = personas
        .iter()
        .find(|p| p.name == "Personal")
        .expect("Personal")
        .clone();
    let work = personas
        .iter()
        .find(|p| p.name == "Work")
        .expect("Work")
        .clone();
    let release = r.name_node("Release 1.2").expect("'Release 1.2'");
    let color = |r: &Rig| {
        r.tree
            .view()
            .rows
            .iter()
            .find(|x| x.node == release)
            .and_then(|x| x.persona_color.clone())
    };
    assert_eq!(color(&r).as_deref(), Some(work.color.as_str()));
    let Some(NodeId::Folder(clients)) = r.name_node("Clients") else {
        panic!("no 'Clients'")
    };
    let done = r
        .http
        .patch_folder(clients, json!({"persona_id": home.id}))
        .await;
    assert!(done.is_ok());
    r.pump("the dot changed", |t| {
        t.view()
            .rows
            .iter()
            .find(|x| x.node == release)
            .and_then(|x| x.persona_color.clone())
            .as_deref()
            == Some(home.color.as_str())
    })
    .await;
    let thread = r.name_node("Review").expect("thread 'Review'");
    assert_eq!(
        r.tree
            .view()
            .rows
            .iter()
            .find(|x| x.node == thread)
            .and_then(|x| x.persona_color.clone())
            .as_deref(),
        Some(home.color.as_str()),
        "the thread follows"
    );
    let stored: Option<i64> =
        r.d.db()
            .reader()
            .expect("reader")
            .query_row(
                "SELECT persona_id FROM sessions WHERE name = 'Release 1.2'",
                [],
                |x| x.get(0),
            )
            .expect("row");
    assert_eq!(stored, None, "the persona is not copied into the session");
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_inherits_after_parent_change() {
    let mut r = rig().await;
    let Some(NodeId::Session(release)) = r.name_node("Release 1.2") else {
        panic!("no 'Release 1.2'")
    };
    let Some(NodeId::Session(thread)) = r.name_node("Review") else {
        panic!("no 'Review'")
    };
    assert_eq!(
        r.tree.session_info(thread).and_then(|i| i.path).as_deref(),
        Some("/home/user/code/release")
    );
    assert!(
        r.http
            .patch_session(release, json!({"path": "/srv/changed"}))
            .await
            .is_ok()
    );
    r.pump("thread path updated", |t| {
        t.session_info(thread).and_then(|i| i.path).as_deref() == Some("/srv/changed")
    })
    .await;
    assert!(
        r.http
            .patch_session(thread, json!({"cfg": {"ctx": "none"}}))
            .await
            .is_ok()
    );
    r.pump("context isolated", |t| {
        t.session_info(thread)
            .is_some_and(|i| !i.context_from_parent)
    })
    .await;
    assert_eq!(
        r.tree.session_info(thread).and_then(|i| i.path).as_deref(),
        Some("/srv/changed"),
        "the path stays inherited"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn move_and_rename_roundtrip_keep_the_other_field() {
    let mut r = rig().await;
    let Some(NodeId::Session(s5)) = r.name_node("Client emails") else {
        panic!("no 'Client emails'")
    };
    let Some(NodeId::Folder(home)) = r.name_node("Home") else {
        panic!("no 'Home'")
    };
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Move { session: s5 }),
        Instant::now(),
    );
    r.run(fx).await;
    r.dlg(DialogMsg::SetFolder(Some(home))).await;
    assert_eq!(r.dlg(DialogMsg::Submit).await, vec![Ok(())]);
    r.pump("moved", |t| {
        t.view()
            .rows
            .iter()
            .position(|x| x.node == s(s5))
            .zip(t.view().rows.iter().position(|x| x.node == f(home)))
            .is_some_and(|(a, b)| a > b)
    })
    .await;
    assert_eq!(r.name_node("Client emails"), Some(s(s5)), "same name");
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Rename { node: s(s5) }),
        Instant::now(),
    );
    r.run(fx).await;
    r.dlg(DialogMsg::SetName("Emails".to_string())).await;
    assert_eq!(r.dlg(DialogMsg::Submit).await, vec![Ok(())]);
    r.pump("renamed", |t| {
        t.view()
            .rows
            .iter()
            .any(|x| x.node == s(s5) && x.name == "Emails")
    })
    .await;
    let folder: Option<i64> =
        r.d.db()
            .reader()
            .expect("reader")
            .query_row("SELECT folder_id FROM sessions WHERE id = ?1", [s5], |x| {
                x.get(0)
            })
            .expect("row");
    assert_eq!(folder, Some(home), "renaming does not move");
}
