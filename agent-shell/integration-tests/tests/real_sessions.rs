//! Session tree scenarios of the GUI against a real `agentd` -- the same as in `sessions.rs` against
//! `fake-agentd`: the `sessions` topic snapshot, creating via a dialog, deleting a folder (contents go to the parent), deleting
//! a session with the neighbour selected. Data is created through the core API, DB keys are read from the real `sessions.db`.
mod real_rig;

use std::time::Instant;

use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg, TreeRequest};
use controllers::tree_types::NodeId;
use controllers::{BusEvent, Controller, Effect};
use core_client::{CoreClient, CoreEvent, HttpClient};
use real_rig::{Real, WAIT};
use serde_json::{Value, json};
use ui::tree_exec::execute;
use ui::tree_feed::{TOPICS, apply_personas, route};

struct Seed {
    clients: i64,
    projects: i64,
    release: i64,
    vacation: i64,
}

struct Rig {
    r: Real,
    _c: CoreClient,
    http: HttpClient,
    tree: SessionTreeController,
    dialog: DialogController,
    rx: async_channel::Receiver<CoreEvent>,
    seed: Seed,
}

fn id_of(v: &Value) -> i64 {
    v["id"].as_i64().expect("id")
}

async fn rig() -> Rig {
    let r = Real::start().await;
    let clients = id_of(&r.post("/folders", json!({"name": "Clients"})).await);
    let projects = id_of(
        &r.post(
            "/folders",
            json!({"name": "Projects", "parent_id": clients}),
        )
        .await,
    );
    let release = id_of(
        &r.post(
            "/sessions",
            json!({"name": "Release 1.2", "folder_id": projects}),
        )
        .await,
    );
    let vacation = id_of(&r.post("/sessions", json!({"name": "Vacation"})).await);
    let c = r.client();
    let rx = c.events();
    for t in TOPICS {
        assert!(c.subscribe(t).is_ok());
    }
    let http = r.http.clone();
    let mut rig = Rig {
        r,
        _c: c,
        http,
        tree: SessionTreeController::new(),
        dialog: DialogController::new(),
        rx,
        seed: Seed {
            clients,
            projects,
            release,
            vacation,
        },
    };
    let list = rig.http.list_personas().await.unwrap_or_default();
    apply_personas(&mut rig.tree, &list, Instant::now());
    rig.pump("sessions snapshot", |t| t.view().rows.len() >= 4)
        .await;
    rig
}

impl Rig {
    async fn pump(&mut self, what: &str, done: impl Fn(&SessionTreeController) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(&self.tree) {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                panic!(
                    "timed out waiting for: {what}; rows: {:?}",
                    self.tree
                        .view()
                        .rows
                        .iter()
                        .map(|r| (r.node, r.name.clone()))
                        .collect::<Vec<_>>()
                )
            };
            route(&mut self.tree, &ev, Instant::now());
        }
    }

    async fn run(&mut self, effects: Vec<Effect>) -> Vec<Result<(), String>> {
        let mut pending = effects;
        let mut results = Vec::new();
        while let Some(e) = pending.pop() {
            match e {
                Effect::Publish(BusEvent::OpenForm { form, opener }) => {
                    pending.extend(
                        self.dialog
                            .update(DialogMsg::OpenForm { form, opener }, Instant::now()),
                    );
                }
                Effect::Command(cmd) => results.push(
                    execute(&self.http, &cmd)
                        .await
                        .unwrap_or_else(|| Err(format!("command not executed: {cmd:?}"))),
                ),
                _ => {}
            }
        }
        results
    }

    async fn tree_msg(&mut self, msg: TreeMsg) -> Vec<Result<(), String>> {
        let fx = self.tree.update(msg, Instant::now());
        self.run(fx).await
    }

    async fn dlg(&mut self, msg: DialogMsg) -> Vec<Result<(), String>> {
        let fx = self.dialog.update(msg, Instant::now());
        self.run(fx).await
    }

    fn depth_of(&self, node: NodeId) -> Option<usize> {
        self.tree
            .view()
            .rows
            .iter()
            .find(|x| x.node == node)
            .map(|x| x.depth)
    }

    fn db_ids(&self, table: &str) -> Vec<i64> {
        let c = rusqlite::Connection::open_with_flags(
            &self.r.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("db");
        let mut st = c
            .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
            .expect("sql");
        st.query_map([], |x| x.get(0))
            .expect("query")
            .flatten()
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_syncs_with_real_agentd() {
    let r = rig().await;
    let view = r.tree.view();
    let mut folders: Vec<i64> = Vec::new();
    let mut sessions: Vec<i64> = Vec::new();
    for row in &view.rows {
        match row.node {
            NodeId::Folder(i) => folders.push(i),
            NodeId::Session(i) => sessions.push(i),
        }
    }
    folders.sort_unstable();
    sessions.sort_unstable();
    assert_eq!(folders, r.db_ids("folders"), "tree folders match the DB");
    assert_eq!(sessions, r.db_ids("sessions"), "tree sessions match the DB");
    assert_eq!(
        r.depth_of(NodeId::Session(r.seed.release)),
        r.depth_of(NodeId::Folder(r.seed.projects)).map(|d| d + 1),
        "nesting is visible"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_session_roundtrip() {
    let mut r = rig().await;
    let open = r.seed.vacation;
    r.tree_msg(TreeMsg::Select(open)).await;
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::NewSession {
            folder: Some(r.seed.projects),
        }),
        Instant::now(),
    );
    r.run(fx).await;
    r.dlg(DialogMsg::SetName("New via dialog".to_string()))
        .await;
    assert_eq!(
        r.dlg(DialogMsg::Submit).await,
        vec![Ok(())],
        "the core accepted"
    );
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
    assert_eq!(
        Some(new.depth),
        r.depth_of(NodeId::Folder(r.seed.projects)).map(|d| d + 1),
        "in the right folder"
    );
    assert_eq!(v.selected, Some(open), "the selection did not slip");
    assert!(
        r.dlg(DialogMsg::Submit).await.is_empty(),
        "nothing to send after the dialog closes"
    );
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
async fn delete_folder_roundtrip_content_goes_to_parent() {
    let mut r = rig().await;
    let (clients, projects, release) = (r.seed.clients, r.seed.projects, r.seed.release);
    r.tree_msg(TreeMsg::Select(release)).await;
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Delete {
            node: NodeId::Folder(projects),
        }),
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
    r.pump("folder deleted, session under 'Clients'", |t| {
        let v = t.view();
        !v.rows.iter().any(|x| x.node == NodeId::Folder(projects))
            && v.rows
                .iter()
                .find(|x| x.node == NodeId::Session(release))
                .is_some_and(|x| x.depth == 1)
    })
    .await;
    assert_eq!(
        r.tree.view().selected,
        Some(release),
        "the selection stayed"
    );
    let c =
        rusqlite::Connection::open_with_flags(&r.r.db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("db");
    let folder: Option<i64> = c
        .query_row(
            "SELECT folder_id FROM sessions WHERE id = ?1",
            [release],
            |x| x.get(0),
        )
        .expect("row");
    assert_eq!(
        folder,
        Some(clients),
        "in the DB the session is in the parent folder"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_roundtrip_selects_neighbour() {
    let mut r = rig().await;
    let release = r.seed.release;
    r.tree_msg(TreeMsg::Select(release)).await;
    let fx = r.tree.update(
        TreeMsg::Request(TreeRequest::Delete {
            node: NodeId::Session(release),
        }),
        Instant::now(),
    );
    r.run(fx).await;
    assert_eq!(r.dlg(DialogMsg::Confirm).await, vec![Ok(())]);
    r.pump("session deleted", |t| {
        t.view().rows.iter().all(|x| x.name != "Release 1.2")
    })
    .await;
    let v = r.tree.view();
    assert!(
        v.selected.is_some_and(|id| id != release),
        "{:?}",
        v.selected
    );
    assert!(!r.db_ids("sessions").contains(&release));
}
