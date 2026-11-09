//! `fake-agentd` tree API, folders: create, edit, move, delete (db.md section 5).

mod tree_common;

use serde_json::json;
use tree_common::*;

// ---------- folders ----------

#[tokio::test(flavor = "multi_thread")]
async fn folder_create_nested_and_validation() {
    let r = rig().await;
    let top = r.folder("Top", None, None).await;
    let sub = r
        .created(
            "/v1/folders",
            json!({"name": "  Nested  ", "parent_id": top}),
        )
        .await;
    assert_eq!(sub["parent_id"], top);
    assert_eq!(sub["name"], "Nested", "the name is trimmed at the edges");
    assert_eq!(sub["open"], true);
    assert!(sub["created_at"].as_i64().unwrap_or(0) > 1_700_000_000);
    for (body, field) in [
        (json!({"name": ""}), "name"),
        (json!({"name": "   "}), "name"),
        (json!({}), "name"),
        (json!({"name": "x", "parent_id": 99_999}), "parent_id"),
        (json!({"name": "x", "persona_id": 99_999}), "persona_id"),
    ] {
        let bad = r.post("/v1/folders", body.clone()).await;
        assert_eq!(bad.status, 422, "{body}");
        assert_eq!(code(&bad), "validation_failed");
        assert_eq!(bad.json()["error"]["details"]["fields"][0], field, "{body}");
    }
    let mut parent = top;
    for i in 0..7 {
        parent = r.folder(&format!("L{i}"), Some(parent), None).await;
    }
    let too_deep = r
        .post("/v1/folders", json!({"name": "9", "parent_id": parent}))
        .await;
    assert_eq!(too_deep.status, 422, "9th level");
    assert_eq!(
        too_deep.json()["error"]["details"]["fields"][0],
        "parent_id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_patch_rename_move_persona_open() {
    let r = rig().await;
    let a = r.folder("A", None, None).await;
    let b = r.folder("B", None, None).await;
    let p = r.persona_id("Work").await;
    let renamed = r
        .patch(&format!("/v1/folders/{a}"), json!({"name": "A2"}))
        .await;
    assert_eq!(renamed.status, 200);
    assert_eq!(renamed.json()["name"], "A2");
    assert!(
        renamed.json()["parent_id"].is_null(),
        "renaming does not move"
    );
    let moved = r
        .patch(&format!("/v1/folders/{a}"), json!({"parent_id": b}))
        .await
        .json();
    assert_eq!(moved["parent_id"], b);
    assert_eq!(moved["name"], "A2", "moving does not rename");
    let pers = r
        .patch(&format!("/v1/folders/{a}"), json!({"persona_id": p}))
        .await
        .json();
    assert_eq!(pers["persona_id"], p);
    let cleared = r
        .patch(&format!("/v1/folders/{a}"), json!({"persona_id": null}))
        .await
        .json();
    assert!(cleared["persona_id"].is_null(), "null clears the persona");
    let closed = r
        .patch(&format!("/v1/folders/{a}"), json!({"open": false}))
        .await
        .json();
    assert_eq!(closed["open"], false);
    assert_eq!(
        r.patch("/v1/folders/99999", json!({"name": "x"}))
            .await
            .status,
        404
    );
    assert_eq!(
        r.patch(&format!("/v1/folders/{a}"), json!({"name": ""}))
            .await
            .status,
        422
    );
    assert_eq!(
        r.patch(&format!("/v1/folders/{a}"), json!({"persona_id": 99_999}))
            .await
            .status,
        422
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_move_into_self_or_descendant_is_conflict() {
    let r = rig().await;
    let a = r.folder("A", None, None).await;
    let b = r.folder("B", Some(a), None).await;
    for target in [a, b] {
        let bad = r
            .patch(&format!("/v1/folders/{a}"), json!({"parent_id": target}))
            .await;
        assert_eq!(bad.status, 409, "into {target}");
        assert_eq!(code(&bad), "conflict");
    }
    let still = r.get("/v1/folders").await;
    assert!(
        by_name(&still, "A")["parent_id"].is_null(),
        "the folder did not move"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_delete_moves_children_up() {
    let r = rig().await;
    let top = r.folder("Top", None, None).await;
    let mid = r.folder("Middle", Some(top), None).await;
    let sub = r.folder("Subfolder", Some(mid), None).await;
    let s_mid = r.session("in middle", Some(mid)).await;
    let t = r
        .created(
            &format!("/v1/sessions/{s_mid}/threads"),
            json!({"name": "thread"}),
        )
        .await;
    let del = r.call("DELETE", &format!("/v1/folders/{mid}"), None).await;
    assert_eq!(del.status, 204);
    let folders = r.get("/v1/folders").await;
    assert!(by_name(&folders, "Middle").is_null(), "row deleted");
    assert_eq!(
        by_name(&folders, "Subfolder")["parent_id"],
        top,
        "the subfolder is at the parent"
    );
    let s = r.session_json(s_mid).await;
    assert_eq!(s["folder_id"], top, "the session is at the parent");
    let th = r.session_json(t["id"].as_i64().unwrap_or(-1)).await;
    assert_eq!(th["folder_id"], top, "the thread follows the parent");
    assert_eq!(by_name(&folders, "Subfolder")["id"], sub, "rows not lost");
    assert_eq!(
        r.call("DELETE", &format!("/v1/folders/{mid}"), None)
            .await
            .status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_delete_top_level_moves_children_to_root() {
    let r = rig().await;
    let top = r.folder("Root", None, None).await;
    r.folder("Child", Some(top), None).await;
    let s = r.session("inside", Some(top)).await;
    assert_eq!(
        r.call("DELETE", &format!("/v1/folders/{top}"), None)
            .await
            .status,
        204
    );
    let folders = r.get("/v1/folders").await;
    assert!(by_name(&folders, "Child")["parent_id"].is_null());
    assert!(r.session_json(s).await["folder_id"].is_null());
}
