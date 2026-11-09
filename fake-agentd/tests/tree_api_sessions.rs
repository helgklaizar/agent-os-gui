//! `fake-agentd` tree API, sessions and threads: create, move, rename, cascading delete.

mod tree_common;

use serde_json::json;
use tree_common::*;

// ---------- sessions and threads ----------

#[tokio::test(flavor = "multi_thread")]
async fn session_create_fields_and_validation() {
    let r = rig().await;
    let f = r.folder("F", None, None).await;
    let s = r
        .created(
            "/v1/sessions",
            json!({"name": " Release ", "folder_id": f, "path": "/home/user/code/x"}),
        )
        .await;
    assert_eq!(s["name"], "Release");
    assert_eq!(s["folder_id"], f);
    assert_eq!(s["path"], "/home/user/code/x");
    assert_eq!(s["unread"], 0);
    assert_eq!(s["archived"], false);
    assert!(s["parent_id"].is_null());
    for body in [
        json!({"name": ""}),
        json!({}),
        json!({"name": "x", "folder_id": 99_999}),
    ] {
        let bad = r.post("/v1/sessions", body.clone()).await;
        assert_eq!(bad.status, 422, "{body}");
        assert_eq!(code(&bad), "validation_failed");
    }
    let before = r.get("/v1/sessions").await.as_array().map_or(0, Vec::len);
    r.post("/v1/sessions", json!({"name": ""})).await;
    assert_eq!(
        r.get("/v1/sessions").await.as_array().map_or(0, Vec::len),
        before,
        "an empty name created nothing"
    );
    assert_eq!(r.call("GET", "/v1/sessions/99999", None).await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_inherits_persona_path_ctx_via_api() {
    let r = rig().await;
    let work = r.persona_id("Work").await;
    let f = r.folder("F", None, Some(work)).await;
    let s = r
        .created(
            "/v1/sessions",
            json!({"name": "parent", "folder_id": f, "path": "/srv/a"}),
        )
        .await;
    let sid = s["id"].as_i64().unwrap_or(-1);
    let t = r
        .created(
            &format!("/v1/sessions/{sid}/threads"),
            json!({"name": "thread"}),
        )
        .await;
    let tid = t["id"].as_i64().unwrap_or(-1);
    assert_eq!(t["parent_id"], sid);
    assert_eq!(t["folder_id"], f, "the parent's folder");
    assert_eq!(t["effective_persona_id"], work);
    assert_eq!(t["path"], "/srv/a", "the parent's path");
    assert_eq!(t["cfg"]["ctx"], "parent", "the parent's context by default");
    r.patch(&format!("/v1/sessions/{sid}"), json!({"path": "/srv/b"}))
        .await;
    assert_eq!(
        r.session_json(tid).await["path"],
        "/srv/b",
        "the thread takes the path at read time"
    );
    let home = r.persona_id("Personal").await;
    r.patch(&format!("/v1/sessions/{sid}"), json!({"persona_id": home}))
        .await;
    assert_eq!(
        r.session_json(tid).await["effective_persona_id"],
        home,
        "the parent's persona beats the folder"
    );
    let iso = r
        .patch(
            &format!("/v1/sessions/{tid}"),
            json!({"cfg": {"ctx": "none"}}),
        )
        .await
        .json();
    assert_eq!(iso["cfg"]["ctx"], "none");
    assert_eq!(
        iso["path"], "/srv/b",
        "ctx = none does not disable the path"
    );
    assert_eq!(
        iso["effective_persona_id"], home,
        "ctx = none does not disable the persona"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_of_thread_and_missing_parent_rejected() {
    let r = rig().await;
    let s = r.session("s", None).await;
    let t = r
        .created(&format!("/v1/sessions/{s}/threads"), json!({"name": "t"}))
        .await;
    let tid = t["id"].as_i64().unwrap_or(-1);
    let deep = r
        .post(
            &format!("/v1/sessions/{tid}/threads"),
            json!({"name": "tt"}),
        )
        .await;
    assert_eq!(deep.status, 422, "one level of threads");
    assert_eq!(
        r.post("/v1/sessions/99999/threads", json!({"name": "x"}))
            .await
            .status,
        404
    );
    assert_eq!(
        r.post(&format!("/v1/sessions/{s}/threads"), json!({"name": " "}))
            .await
            .status,
        422
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_move_changes_folder_only_and_follows_threads() {
    let r = rig().await;
    let (a, b) = (
        r.folder("A", None, None).await,
        r.folder("B", None, None).await,
    );
    let s = r.session("Name", Some(a)).await;
    let t = r
        .created(
            &format!("/v1/sessions/{s}/threads"),
            json!({"name": "Thread"}),
        )
        .await["id"]
        .as_i64()
        .unwrap_or(-1);
    let moved = r
        .patch(&format!("/v1/sessions/{s}"), json!({"folder_id": b}))
        .await
        .json();
    assert_eq!(moved["folder_id"], b);
    assert_eq!(moved["name"], "Name", "moving does not rename");
    assert_eq!(
        r.session_json(t).await["folder_id"],
        b,
        "the thread follows the parent"
    );
    assert_eq!(r.session_json(t).await["name"], "Thread");
    let out = r
        .patch(&format!("/v1/sessions/{s}"), json!({"folder_id": null}))
        .await
        .json();
    assert!(out["folder_id"].is_null(), "null - \"No folder\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_rename_changes_name_only() {
    let r = rig().await;
    let f = r.folder("A", None, None).await;
    let s = r.session("Old", Some(f)).await;
    let renamed = r
        .patch(&format!("/v1/sessions/{s}"), json!({"name": "New"}))
        .await
        .json();
    assert_eq!(renamed["name"], "New");
    assert_eq!(renamed["folder_id"], f, "renaming does not move");
    assert_eq!(
        r.patch(&format!("/v1/sessions/{s}"), json!({"name": ""}))
            .await
            .status,
        422
    );
    assert_eq!(r.session_json(s).await["name"], "New");
    assert_eq!(
        r.patch("/v1/sessions/99999", json!({"name": "x"}))
            .await
            .status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_cannot_be_moved_alone() {
    let r = rig().await;
    let (a, b) = (
        r.folder("A", None, None).await,
        r.folder("B", None, None).await,
    );
    let s = r.session("s", Some(a)).await;
    let t = r
        .created(&format!("/v1/sessions/{s}/threads"), json!({"name": "t"}))
        .await["id"]
        .as_i64()
        .unwrap_or(-1);
    let bad = r
        .patch(&format!("/v1/sessions/{t}"), json!({"folder_id": b}))
        .await;
    assert_eq!(
        bad.status, 422,
        "the thread follows the parent, does not move itself"
    );
    assert_eq!(r.session_json(t).await["folder_id"], a);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_delete_cascades_threads_only() {
    let r = rig().await;
    let f = r.folder("F", None, None).await;
    let keep = r.session("neighbor", Some(f)).await;
    let gone = r.session("doomed", Some(f)).await;
    let mut tids = Vec::new();
    for i in 0..3 {
        tids.push(
            r.created(
                &format!("/v1/sessions/{gone}/threads"),
                json!({"name": format!("t{i}")}),
            )
            .await["id"]
                .as_i64()
                .unwrap_or(-1),
        );
    }
    assert_eq!(
        r.call("DELETE", &format!("/v1/sessions/{gone}"), None)
            .await
            .status,
        204
    );
    assert_eq!(
        r.call("GET", &format!("/v1/sessions/{gone}"), None)
            .await
            .status,
        404
    );
    for t in tids {
        assert_eq!(
            r.call("GET", &format!("/v1/sessions/{t}"), None)
                .await
                .status,
            404,
            "thread {t}"
        );
    }
    assert_eq!(r.session_json(keep).await["name"], "neighbor");
    assert_eq!(
        r.call("DELETE", &format!("/v1/sessions/{gone}"), None)
            .await
            .status,
        404
    );
    let conn = r.d.db().reader().expect("reader");
    let fk: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |x| {
            x.get(0)
        })
        .unwrap_or(-1);
    assert_eq!(fk, 0);
}
