//! A2-09: GUI S3 scenarios against the real `agentd` through `core-client`: the client keeps its own tree from `snapshot` and
//! `delta` of the `sessions` topic and finally compares it with what HTTP returns.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::{Env, persona};
use core_client::api::Frame;
use serde_json::{Value, json};

/// Client-side tree (what `SessionTreeController` holds).
#[derive(Default)]
struct Tree {
    folders: BTreeMap<i64, Value>,
    sessions: BTreeMap<i64, Value>,
}

impl Tree {
    fn load(snapshot: &Value) -> Tree {
        let by_id = |k: &str| -> BTreeMap<i64, Value> {
            snapshot[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (v["id"].as_i64().unwrap(), v.clone()))
                .collect()
        };
        Tree {
            folders: by_id("folders"),
            sessions: by_id("sessions"),
        }
    }

    fn apply(&mut self, d: &Value) {
        let id = |v: &Value| v["id"].as_i64().unwrap();
        match d["op"].as_str().unwrap() {
            "folder.created" | "folder.updated" => {
                drop(self.folders.insert(id(&d["folder"]), d["folder"].clone()))
            }
            "folder.deleted" => drop(self.folders.remove(&id(&d["folder"]))),
            "session.created" | "session.updated" => drop(
                self.sessions
                    .insert(id(&d["session"]), d["session"].clone()),
            ),
            "session.deleted" => {
                let gone = id(&d["session"]);
                self.sessions
                    .retain(|k, s| *k != gone && s["parent_id"].as_i64() != Some(gone));
            }
            "unread" => {
                let s = self
                    .sessions
                    .get_mut(&d["session_id"].as_i64().unwrap())
                    .unwrap();
                s["unread"] = d["unread"].clone();
            }
            other => panic!("unknown delta {other}"),
        }
    }

    fn feed(&mut self, frames: &[Frame]) {
        for f in frames {
            self.apply(&f.data);
        }
    }
}

fn effective(t: &Tree, session: &Value) -> Value {
    t.sessions[&session["id"].as_i64().unwrap()]["effective_persona_id"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_s3_tree_scenarios() {
    let e = Env::start_with(|c| {
        c.execute_batch(
            "INSERT INTO sessions(name, unread, created_at, updated_at) VALUES ('foreign', 2, 1, 1);",
        )
    })
    .await;
    let mut ws = e.ws();
    let mut tree = Tree::load(&ws.subscribe("sessions"));
    assert_eq!(tree.sessions.len(), 1);
    let (work, home) = (
        e.admin.make("/personas", persona("Work")).await,
        e.admin.make("/personas", persona("Home")).await,
    );

    // Create dialog: a folder with a persona, a nested folder, sessions with and without a folder.
    let projects = e
        .chat
        .make(
            "/folders",
            json!({"name": "Projects", "persona_id": work["id"]}),
        )
        .await;
    let sub = e
        .chat
        .make(
            "/folders",
            json!({"name": "Agent", "parent_id": projects["id"]}),
        )
        .await;
    let s1 = e
        .chat
        .make("/sessions", json!({"name": "Plan", "folder_id": sub["id"]}))
        .await;
    let s2 = e.chat.make("/sessions", json!({"name": "Free"})).await;
    let thread = e
        .chat
        .make(
            &format!("/sessions/{}/threads", s1["id"]),
            json!({"name": "Branch"}),
        )
        .await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    assert_eq!(
        effective(&tree, &s1),
        work["id"],
        "persona of the nearest folder upwards"
    );
    assert_eq!(effective(&tree, &thread), work["id"], "thread inherits");
    assert_eq!(effective(&tree, &s2), Value::Null);

    // Changing a folder's persona updates the view without recreation (regression: "copied at creation").
    e.chat
        .patch(
            &format!("/folders/{}", projects["id"]),
            json!({"persona_id": home["id"]}),
        )
        .await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    assert_eq!(effective(&tree, &s1), home["id"]);
    assert_eq!(effective(&tree, &thread), home["id"]);

    // Moving does not rename, renaming does not move, a thread follows its parent.
    e.chat
        .patch(
            &format!("/sessions/{}", s1["id"]),
            json!({"folder_id": projects["id"]}),
        )
        .await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    let (moved, kid) = (
        &tree.sessions[&s1["id"].as_i64().unwrap()],
        &tree.sessions[&thread["id"].as_i64().unwrap()],
    );
    assert_eq!(
        (moved["name"].as_str(), moved["folder_id"].clone()),
        (Some("Plan"), projects["id"].clone())
    );
    assert_eq!(kid["folder_id"], projects["id"]);
    e.chat
        .patch(
            &format!("/sessions/{}", s1["id"]),
            json!({"name": "Plan 2"}),
        )
        .await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    assert_eq!(
        tree.sessions[&s1["id"].as_i64().unwrap()]["folder_id"],
        projects["id"]
    );

    // Unread: opening a session resets the badge.
    assert_eq!(tree.sessions[&1]["unread"], 2);
    e.chat.post("/sessions/1/read", json!({})).await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    assert_eq!(tree.sessions[&1]["unread"], 0);

    // Deletion: a session with a thread and a folder with contents.
    e.chat.delete(&format!("/sessions/{}", s1["id"])).await;
    e.chat.delete(&format!("/folders/{}", projects["id"])).await;
    tree.feed(&ws.drain(Duration::from_millis(500)));
    assert!(
        !tree.sessions.contains_key(&thread["id"].as_i64().unwrap()),
        "thread went away with its parent"
    );
    let sub_now = &tree.folders[&sub["id"].as_i64().unwrap()];
    assert_eq!(
        sub_now["parent_id"],
        Value::Null,
        "nested folder moved up one level"
    );

    // Client state equals core state.
    let (_, folders) = e.read.get("/folders").await;
    let (_, sessions) = e.read.get("/sessions").await;
    assert_eq!(
        Value::Array(tree.folders.values().cloned().collect()),
        folders
    );
    let mut core_sessions = sessions.as_array().unwrap().clone();
    core_sessions.sort_by_key(|s| s["id"].as_i64());
    assert_eq!(
        Value::Array(tree.sessions.values().cloned().collect()),
        Value::Array(core_sessions)
    );
}
