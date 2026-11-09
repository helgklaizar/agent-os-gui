//! `fake-agentd` tree API, reading: fixtures, personas, filters, persona inheritance, `POST /sessions/{id}/read`.

mod tree_common;

use serde_json::{Value, json};
use tree_common::*;

// ---------- fixtures and reading ----------

#[tokio::test(flavor = "multi_thread")]
async fn fixtures_have_nested_tree_threads_personas_unread() {
    let r = rig().await;
    let folders = r.get("/v1/folders").await;
    let sessions = r.get("/v1/sessions").await;
    let personas = r.get("/v1/personas").await;
    let nested = folders
        .as_array()
        .into_iter()
        .flatten()
        .any(|f| f["parent_id"].is_i64());
    assert!(nested, "the fixtures have a nested folder: {folders}");
    let threads = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["parent_id"].is_i64())
        .count();
    assert!(threads >= 2, "the fixtures have threads: {sessions}");
    assert!(
        sessions
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["unread"].as_i64().unwrap_or(0) > 0),
        "there are unread"
    );
    for name in ["Personal", "Work"] {
        let p = by_name(&personas, name);
        assert!(p.is_object(), "no persona {name}: {personas}");
        assert!(
            p["account_ids"].as_array().is_some_and(|a| !a.is_empty()),
            "{name} has accounts from migration 0002: {p}"
        );
    }
    assert!(
        sessions
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["persona_id"].is_null() && s["effective_persona_id"].is_i64()),
        "there is a session without its own persona but with an inherited one: {sessions}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn personas_are_created_since_s7() {
    let r = rig().await;
    let created = r.post("/v1/personas", json!({"name": "x"})).await;
    assert_eq!(created.status, 201, "creating personas");
    let list = r.get("/v1/personas").await;
    assert!(list.as_array().is_some_and(|a| a.len() >= 3));
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_list_filters() {
    let r = rig().await;
    let f = r.folder("Filter", None, None).await;
    let s = r.session("in folder", Some(f)).await;
    r.created(
        &format!("/v1/sessions/{s}/threads"),
        json!({"name": "thread"}),
    )
    .await;
    let in_folder = r.get(&format!("/v1/sessions?folder_id={f}")).await;
    assert_eq!(
        in_folder.as_array().map(Vec::len),
        Some(2),
        "a session and its thread: {in_folder}"
    );
    let threads = r.get(&format!("/v1/sessions?parent_id={s}")).await;
    assert_eq!(threads.as_array().map(Vec::len), Some(1));
    assert_eq!(threads[0]["name"], "thread");
    r.patch(&format!("/v1/sessions/{s}"), json!({"archived": true}))
        .await;
    let live = r.get(&format!("/v1/sessions?folder_id={f}")).await;
    assert_eq!(
        live.as_array().map(Vec::len),
        Some(1),
        "an archived one is not in the default list: {live}"
    );
    let arch = r.get("/v1/sessions?archived=true").await;
    assert!(
        arch.as_array()
            .is_some_and(|a| a.iter().any(|x| x["id"] == s)),
        "{arch}"
    );
}

// ---------- persona: computed at read time ----------

#[tokio::test(flavor = "multi_thread")]
async fn persona_inherited_dynamically_via_api() {
    let r = rig().await;
    let (work, home) = (r.persona_id("Work").await, r.persona_id("Personal").await);
    let f = r.folder("Folder", None, Some(work)).await;
    let inner = r.folder("Inside", Some(f), None).await;
    let s = r.session("no own", Some(inner)).await;
    let first = r.session_json(s).await;
    assert!(first["persona_id"].is_null());
    assert_eq!(first["effective_persona_id"], work, "nearest folder upward");
    r.patch(&format!("/v1/folders/{f}"), json!({"persona_id": home}))
        .await;
    let after = r.session_json(s).await;
    assert_eq!(
        after["effective_persona_id"], home,
        "changing a folder's persona is visible without recreation"
    );
    assert!(
        after["persona_id"].is_null(),
        "the persona is not copied into the session"
    );
    r.patch(&format!("/v1/folders/{inner}"), json!({"persona_id": work}))
        .await;
    assert_eq!(
        r.session_json(s).await["effective_persona_id"],
        work,
        "the nearer beats the farther"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_own_overrides_folder_via_api() {
    let r = rig().await;
    let (work, home) = (r.persona_id("Work").await, r.persona_id("Personal").await);
    let f = r.folder("Folder", None, Some(work)).await;
    let s = r
        .created(
            "/v1/sessions",
            json!({"name": "own", "folder_id": f, "persona_id": home}),
        )
        .await;
    let id = s["id"].as_i64().unwrap_or(-1);
    assert_eq!(s["effective_persona_id"], home);
    r.patch(&format!("/v1/folders/{f}"), json!({"persona_id": null}))
        .await;
    assert_eq!(
        r.session_json(id).await["effective_persona_id"],
        home,
        "the folder does not touch its own"
    );
    let back = r
        .patch(&format!("/v1/sessions/{id}"), json!({"persona_id": null}))
        .await
        .json();
    assert!(
        back["effective_persona_id"].is_null(),
        "no persona anywhere - null: {back}"
    );
    let bad = r
        .post("/v1/sessions", json!({"name": "x", "persona_id": 99_999}))
        .await;
    assert_eq!(bad.status, 422);
}

// ---------- unread ----------

#[tokio::test(flavor = "multi_thread")]
async fn session_read_route_resets_unread() {
    let r = rig().await;
    let list = r.get("/v1/sessions").await;
    let s = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["unread"] == 3)
        .cloned()
        .unwrap_or(Value::Null);
    assert!(
        s.is_object(),
        "the fixtures have a session with unread = 3: {list}"
    );
    let id = s["id"].as_i64().unwrap_or(-1);
    let read = r.post(&format!("/v1/sessions/{id}/read"), json!({})).await;
    assert_eq!(read.status, 204);
    assert!(read.body.is_empty());
    assert_eq!(r.session_json(id).await["unread"], 0);
    assert_eq!(
        r.post("/v1/sessions/99999/read", json!({})).await.status,
        404
    );
    assert_eq!(
        r.post(&format!("/v1/sessions/{id}/read"), json!({}))
            .await
            .status,
        204,
        "a repeat is harmless"
    );
}
