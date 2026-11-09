//! A2-04: folders (`/folders`): nesting, cycle and depth, move and rename separately.

mod common;

use common::{Env, fields, persona};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn folder_create_list_patch() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", persona("p")).await;
    let f = e.chat.make("/folders", json!({"name": "Projects"})).await;
    assert_eq!(
        (
            f["parent_id"].clone(),
            f["persona_id"].clone(),
            f["open"].clone()
        ),
        (Value::Null, Value::Null, json!(true))
    );
    assert!(f["created_at"].as_i64().unwrap() > 0);
    let child = e
        .chat
        .make(
            "/folders",
            json!({"name": "a", "parent_id": f["id"], "persona_id": p["id"], "open": false}),
        )
        .await;
    assert_eq!(
        (
            child["parent_id"].clone(),
            child["persona_id"].clone(),
            child["open"].clone()
        ),
        (f["id"].clone(), p["id"].clone(), json!(false))
    );
    let (st, list) = e.read.get("/folders").await;
    assert_eq!((st, list.as_array().unwrap().len()), (200, 2));
    let path = format!("/folders/{}", child["id"]);
    let (st, v) = e
        .chat
        .patch(&path, json!({"open": true, "persona_id": null}))
        .await;
    assert_eq!(
        (st, v["open"].clone(), v["persona_id"].clone()),
        (200, json!(true), Value::Null)
    );
    assert_eq!(v["parent_id"], f["id"], "the rest is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_validation() {
    let e = Env::start().await;
    let f = e.chat.make("/folders", json!({"name": "a"})).await;
    for (body, field) in [
        (json!({"name": ""}), "name"),
        (json!({"name": "x".repeat(201)}), "name"),
        (json!({"name": "a\0"}), "name"),
        (json!({}), "name"),
        (json!({"name": 5}), "name"),
        (json!({"name": "a", "parent_id": 999}), "parent_id"),
        (json!({"name": "a", "persona_id": 999}), "persona_id"),
        (json!({"name": "a", "open": "yes"}), "open"),
        (json!({"name": "a", "color": "red"}), "color"),
    ] {
        let (st, v) = e.chat.post("/folders", body.clone()).await;
        assert_eq!(st, 422, "{body}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{body}: {:?}",
            fields(&v)
        );
    }
    let path = format!("/folders/{}", f["id"]);
    assert_eq!(e.chat.patch(&path, json!({"parent_id": 999})).await.0, 422);
    assert_eq!(
        e.chat.patch("/folders/999", json!({"name": "x"})).await.0,
        404
    );
    assert_eq!(e.chat.delete("/folders/999").await.0, 404);
    assert_eq!(
        e.chat.patch("/folders/abc", json!({"name": "x"})).await.0,
        400
    );
    assert_eq!(e.read.post("/folders", json!({"name": "x"})).await.0, 403);
    assert_eq!(e.read.patch(&path, json!({"name": "x"})).await.0, 403);
    assert_eq!(e.read.delete(&path).await.0, 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_cycle_aborts() {
    let e = Env::start().await;
    let a = e.chat.make("/folders", json!({"name": "a"})).await;
    let b = e
        .chat
        .make("/folders", json!({"name": "b", "parent_id": a["id"]}))
        .await;
    let c = e
        .chat
        .make("/folders", json!({"name": "c", "parent_id": b["id"]}))
        .await;
    // Onto itself.
    let (st, v) = e
        .chat
        .patch(
            &format!("/folders/{}", a["id"]),
            json!({"parent_id": a["id"]}),
        )
        .await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (409, Some("conflict")),
        "{v}"
    );
    // Into a descendant.
    let (st, v) = e
        .chat
        .patch(
            &format!("/folders/{}", a["id"]),
            json!({"parent_id": c["id"]}),
        )
        .await;
    assert_eq!(
        (
            st,
            v["error"]["code"].as_str(),
            v["error"]["details"]["reason"].as_str()
        ),
        (409, Some("conflict"), Some("folder_cycle"))
    );
    // Whole rollback: the name from the same PATCH was not applied.
    let (st, _) = e
        .chat
        .patch(
            &format!("/folders/{}", a["id"]),
            json!({"name": "zzz", "parent_id": c["id"]}),
        )
        .await;
    assert_eq!(st, 409);
    let (_, list) = e.chat.get("/folders").await;
    assert_eq!(list[0]["name"], "a");
    assert_eq!(list[0]["parent_id"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_depth_limit_8() {
    let e = Env::start().await;
    let mut parent = Value::Null;
    for i in 0..8 {
        let f = e
            .chat
            .make(
                "/folders",
                json!({"name": format!("d{i}"), "parent_id": parent}),
            )
            .await;
        parent = f["id"].clone();
    }
    let (st, v) = e
        .chat
        .post("/folders", json!({"name": "d8", "parent_id": parent}))
        .await;
    assert_eq!(st, 422, "ninth level: {v}");
    assert_eq!(v["error"]["code"], "validation_failed");
    assert!(fields(&v).contains(&"parent_id".to_string()), "{v}");
    // Moving a folder under the deepest level is also rejected.
    let lone = e.chat.make("/folders", json!({"name": "lone"})).await;
    let (st, v) = e
        .chat
        .patch(
            &format!("/folders/{}", lone["id"]),
            json!({"parent_id": parent}),
        )
        .await;
    assert_eq!(st, 422, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn move_does_not_rename_and_rename_does_not_move() {
    let e = Env::start().await;
    let a = e.chat.make("/folders", json!({"name": "a"})).await;
    let b = e.chat.make("/folders", json!({"name": "b"})).await;
    let path = format!("/folders/{}", b["id"]);
    let (_, moved) = e.chat.patch(&path, json!({"parent_id": a["id"]})).await;
    assert_eq!(
        (moved["name"].as_str(), moved["parent_id"].clone()),
        (Some("b"), a["id"].clone()),
        "move did not rename"
    );
    let (_, renamed) = e.chat.patch(&path, json!({"name": "b2"})).await;
    assert_eq!(
        (renamed["name"].as_str(), renamed["parent_id"].clone()),
        (Some("b2"), a["id"].clone()),
        "rename did not move"
    );
    let (_, up) = e.chat.patch(&path, json!({"parent_id": null})).await;
    assert_eq!(up["parent_id"], Value::Null, "null means the root");
}
