//! Tree and dialogs against `fake-agentd` over a real socket: the `sessions` subscription, creating and deleting.

mod common;
mod sessions_common;

use std::time::Instant;

use controllers::Controller;
use controllers::controllers::dialog::DialogMsg;
use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::tree_types::NodeId;
use sessions_common::*;

#[tokio::test(flavor = "multi_thread")]
async fn tree_syncs_with_fake_agentd() {
    let r = rig().await;
    let view = r.tree.view();
    let mut folders: Vec<i64> = view
        .rows
        .iter()
        .filter_map(|x| {
            if let NodeId::Folder(i) = x.node {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    let mut sessions: Vec<i64> = view
        .rows
        .iter()
        .filter_map(|x| {
            if let NodeId::Session(i) = x.node {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    folders.sort_unstable();
    sessions.sort_unstable();
    assert_eq!(folders, r.db_ids("folders"), "tree folders match the DB");
    assert_eq!(
        sessions,
        r.db_ids("sessions"),
        "tree sessions and threads match the DB"
    );
    assert!(view.rows.iter().any(|x| x.depth >= 2), "nesting is visible");
    let release = r.name_node("Release 1.2").expect("fixture 'Release 1.2'");
    assert!(
        r.tree
            .view()
            .rows
            .iter()
            .find(|x| x.node == release)
            .is_some_and(|x| x.persona_color.is_some()),
        "effective persona from folders"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_session_roundtrip() {
    let mut r = rig().await;
    let Some(NodeId::Session(open_id)) = r.name_node("Vacation") else {
        panic!("no 'Vacation'")
    };
    r.tree_msg(TreeMsg::Select(open_id)).await;
    let projects = match r.name_node("Projects") {
        Some(NodeId::Folder(i)) => i,
        other => panic!("no 'Projects': {other:?}"),
    };
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::NewSession {
            folder: Some(projects),
        }),
        Instant::now(),
    );
    r.run(fx).await;
    r.dlg(DialogMsg::SetName("New via dialog".to_string()))
        .await;
    let results = r.dlg(DialogMsg::Submit).await;
    assert_eq!(results, vec![Ok(())], "the core accepted the command");
    r.pump("node in 'Projects'", |t| {
        t.view().rows.iter().any(|x| x.name == "New via dialog")
    })
    .await;
    let v = r.tree.view();
    let new = v
        .rows
        .iter()
        .find(|x| x.name == "New via dialog")
        .expect("row");
    let folder_depth = v
        .rows
        .iter()
        .find(|x| x.node == f(projects))
        .map(|x| x.depth)
        .unwrap_or(0);
    assert_eq!(new.depth, folder_depth + 1, "in the right folder");
    assert_eq!(v.selected, Some(open_id), "the selection did not slip");
    let empty = r.dlg(DialogMsg::Submit).await;
    assert!(empty.is_empty(), "nothing to send after the dialog closes");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_with_empty_name_sends_nothing() {
    let mut r = rig().await;
    let before = r.db_ids("sessions").len();
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::NewSession { folder: None }),
        Instant::now(),
    );
    r.run(fx).await;
    assert!(r.dlg(DialogMsg::Submit).await.is_empty());
    assert_eq!(
        r.db_ids("sessions").len(),
        before,
        "nothing appeared in the DB"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_folder_roundtrip() {
    let mut r = rig().await;
    // delete the nested folder 'Projects' (parent 'Clients'): the contents must go to 'Clients', not to the root
    let Some(NodeId::Folder(clients)) = r.name_node("Clients") else {
        panic!("no 'Clients'")
    };
    let Some(NodeId::Folder(projects)) = r.name_node("Projects") else {
        panic!("no 'Projects'")
    };
    let Some(NodeId::Session(release)) = r.name_node("Release 1.2") else {
        panic!("no 'Release 1.2'")
    };
    let Some(NodeId::Session(review)) = r.name_node("Review") else {
        panic!("no 'Review'")
    };
    r.tree_msg(TreeMsg::Select(release)).await;
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Delete { node: f(projects) }),
        Instant::now(),
    );
    r.run(fx).await;
    let text = r
        .dialog
        .view()
        .form
        .map(|x| x.lines.join(" "))
        .unwrap_or_default();
    assert!(text.contains("its contents move up one level"), "{text}");
    assert_eq!(r.dlg(DialogMsg::Confirm).await, vec![Ok(())]);
    r.pump("folder deleted", |t| {
        !t.view().rows.iter().any(|x| x.node == f(projects))
    })
    .await;
    // the session is a direct child of 'Clients' (depth 1), its thread one level below; nothing fell into the root
    r.pump("session and thread under 'Clients'", |t| {
        let v = t.view();
        let depth = |n| v.rows.iter().find(|x| x.node == n).map(|x| x.depth);
        depth(s(release)) == Some(1) && depth(s(review)) == Some(2)
    })
    .await;
    let v = r.tree.view();
    let order: Vec<NodeId> = v.rows.iter().map(|x| x.node).collect();
    let at = |n| order.iter().position(|x| *x == n);
    assert!(
        at(f(clients)) < at(s(release)),
        "session after the 'Clients' row"
    );
    assert_eq!(v.selected, Some(release), "the selected session stayed");
    let conn = r.d.db().reader().expect("reader");
    let folder_of = |id: i64| -> Option<i64> {
        conn.query_row("SELECT folder_id FROM sessions WHERE id = ?1", [id], |x| {
            x.get(0)
        })
        .expect("row")
    };
    assert_eq!(
        folder_of(release),
        Some(clients),
        "in the DB the session is in the parent folder"
    );
    assert_eq!(
        folder_of(review),
        Some(clients),
        "the thread follows the parent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_roundtrip_with_threads_and_neighbour_selection() {
    let mut r = rig().await;
    let Some(NodeId::Session(release)) = r.name_node("Release 1.2") else {
        panic!("no 'Release 1.2'")
    };
    r.tree_msg(TreeMsg::Select(release)).await;
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Delete { node: s(release) }),
        Instant::now(),
    );
    r.run(fx).await;
    let text = r
        .dialog
        .view()
        .form
        .map(|x| x.lines.join(" "))
        .unwrap_or_default();
    assert!(
        text.contains("Release 1.2") && text.contains("2 threads"),
        "{text}"
    );
    assert_eq!(r.dlg(DialogMsg::Confirm).await, vec![Ok(())]);
    r.pump("session deleted together with its threads", |t| {
        t.view()
            .rows
            .iter()
            .all(|x| x.name != "Release 1.2" && x.name != "Review" && x.name != "Hotfix")
    })
    .await;
    let v = r.tree.view();
    assert!(
        v.selected.is_some_and(|id| id != release),
        "a neighbour is selected: {:?}",
        v.selected
    );
    assert!(v.selected.is_some_and(|id| r.has(s(id))));
    assert!(!r.db_ids("sessions").contains(&release));
}
