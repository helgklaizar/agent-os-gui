//! A2-05: sessions and threads (`/sessions`): effective persona on read, thread inheritance, `cfg`, path, move.

mod common;

use common::{Env, fields};
use serde_json::{Value, json};

async fn session(e: &Env, body: Value) -> Value {
    e.chat.make("/sessions", body).await
}

async fn get(e: &Env, id: &Value) -> Value {
    let (st, v) = e.read.get(&format!("/sessions/{id}")).await;
    assert_eq!(st, 200, "{v}");
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn session_create_and_get_shape() {
    let e = Env::start().await;
    let s = session(&e, json!({"name": "Chat"})).await;
    for (k, v) in [
        ("folder_id", Value::Null),
        ("parent_id", Value::Null),
        ("persona_id", Value::Null),
        ("effective_persona_id", Value::Null),
        ("path", Value::Null),
        ("cfg", json!({})),
        ("unread", json!(0)),
        ("archived", json!(false)),
        ("name", json!("Chat")),
    ] {
        assert_eq!(s[k], v, "field {k}: {s}");
    }
    assert!(s["created_at"].as_i64().unwrap() > 0 && s["updated_at"].as_i64().unwrap() > 0);
    assert_eq!(get(&e, &s["id"]).await, s);
    assert_eq!(e.read.get("/sessions/999").await.0, 404);
    assert_eq!(e.read.get("/sessions/x").await.0, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn move_does_not_rename() {
    let e = Env::start().await;
    let f = e.chat.make("/folders", json!({"name": "f"})).await;
    let s = session(&e, json!({"name": "Original name"})).await;
    let (_, t) = e
        .chat
        .post(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "thread"}),
        )
        .await;
    let (st, v) = e
        .chat
        .patch(
            &format!("/sessions/{}", s["id"]),
            json!({"folder_id": f["id"]}),
        )
        .await;
    assert_eq!(
        (st, v["name"].as_str(), v["folder_id"].clone()),
        (200, Some("Original name"), f["id"].clone())
    );
    let t_after = get(&e, &t["id"]).await;
    assert_eq!(
        (t_after["folder_id"].clone(), t_after["name"].as_str()),
        (f["id"].clone(), Some("thread")),
        "the thread follows its parent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_does_not_move() {
    let e = Env::start().await;
    let f = e.chat.make("/folders", json!({"name": "f"})).await;
    let s = session(&e, json!({"name": "a", "folder_id": f["id"]})).await;
    let (_, v) = e
        .chat
        .patch(&format!("/sessions/{}", s["id"]), json!({"name": "b"}))
        .await;
    assert_eq!(
        (v["name"].as_str(), v["folder_id"].clone()),
        (Some("b"), f["id"].clone())
    );
    let (_, v) = e
        .chat
        .patch(
            &format!("/sessions/{}", s["id"]),
            json!({"folder_id": null}),
        )
        .await;
    assert_eq!(
        (v["name"].as_str(), v["folder_id"].clone()),
        (Some("b"), Value::Null),
        "null — from the folder to the root"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_fields_validation_and_scope() {
    let e = Env::start().await;
    for (body, field) in [
        (json!({}), "name"),
        (json!({"name": ""}), "name"),
        (json!({"name": "   "}), "name"),
        (json!({"name": "x".repeat(201)}), "name"),
        (json!({"name": "a", "folder_id": 999}), "folder_id"),
        (json!({"name": "a", "persona_id": 999}), "persona_id"),
        (json!({"name": "a", "unread": 5}), "unread"),
        (json!({"name": "a", "parent_id": 1}), "parent_id"),
    ] {
        let (st, v) = e.chat.post("/sessions", body.clone()).await;
        assert_eq!(st, 422, "{body}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{body}: {:?}",
            fields(&v)
        );
    }
    assert_eq!(
        e.chat
            .post("/sessions", json!({"name": "x".repeat(200)}))
            .await
            .0,
        201,
        "exactly 200 is allowed"
    );
    let s = session(&e, json!({"name": "s"})).await;
    let path = format!("/sessions/{}", s["id"]);
    assert_eq!(e.read.post("/sessions", json!({"name": "x"})).await.0, 403);
    assert_eq!(e.read.patch(&path, json!({"name": "x"})).await.0, 403);
    assert_eq!(e.read.delete(&path).await.0, 403);
    assert_eq!(e.read.post(&format!("{path}/read"), json!({})).await.0, 403);
    assert_eq!(
        e.chat.patch("/sessions/999", json!({"name": "x"})).await.0,
        404
    );
    let (st, v) = e
        .chat
        .patch(&path, json!({"archived": true, "name": "z"}))
        .await;
    assert_eq!(
        (st, v["archived"].clone(), v["name"].as_str()),
        (200, json!(true), Some("z"))
    );
    // Atomicity: a bad field in the same PATCH — nothing was applied.
    let (st, _) = e
        .chat
        .patch(&path, json!({"name": "after", "persona_id": 999}))
        .await;
    assert_eq!(st, 422);
    assert_eq!(get(&e, &s["id"]).await["name"], "z");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_list_filters() {
    let e = Env::start().await;
    let f = e.chat.make("/folders", json!({"name": "f"})).await;
    let a = session(&e, json!({"name": "a", "folder_id": f["id"]})).await;
    let b = session(&e, json!({"name": "b"})).await;
    let (_, t) = e
        .chat
        .post(
            &format!("/sessions/{}/threads", a["id"]),
            json!({"name": "t"}),
        )
        .await;
    e.chat
        .patch(&format!("/sessions/{}", b["id"]), json!({"archived": true}))
        .await;
    let ids = |v: Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_i64().unwrap())
            .collect()
    };
    let (a_id, b_id, t_id) = (
        a["id"].as_i64().unwrap(),
        b["id"].as_i64().unwrap(),
        t["id"].as_i64().unwrap(),
    );
    assert_eq!(
        ids(e.read.get("/sessions").await.1),
        [a_id, b_id, t_id],
        "no filters — all rows"
    );
    assert_eq!(
        ids(e
            .read
            .get(&format!("/sessions?folder_id={}", f["id"]))
            .await
            .1),
        [a_id, t_id]
    );
    assert_eq!(
        ids(e.read.get(&format!("/sessions?parent_id={a_id}")).await.1),
        [t_id]
    );
    assert_eq!(ids(e.read.get("/sessions?archived=true").await.1), [b_id]);
    assert_eq!(
        ids(e.read.get("/sessions?archived=false").await.1),
        [a_id, t_id]
    );
    assert_eq!(e.read.get("/sessions?folder_id=abc").await.0, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn evil_names_stored_verbatim() {
    let e = Env::start().await;
    let evil = "<img src=x onerror=alert(1)> **bold** {name} %s \u{202e}rtl";
    let s = session(&e, json!({"name": evil})).await;
    assert_eq!(
        get(&e, &s["id"]).await["name"],
        evil,
        "markup is stored as is"
    );
    let f = e.chat.make("/folders", json!({"name": evil})).await;
    assert_eq!(f["name"], evil);
    for bad in ["a\0b".to_string(), "x".repeat(10_000)] {
        assert_eq!(e.chat.post("/sessions", json!({"name": bad})).await.0, 422);
        assert_eq!(e.chat.post("/folders", json!({"name": bad})).await.0, 422);
    }
}
