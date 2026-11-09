//! A2-05: effective persona on read and inheritance by threads (persona, path, context).

mod common;

use common::{Env, fields, persona};
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
async fn persona_effective_session_own() {
    let e = Env::start().await;
    let (own, other) = (
        e.admin.make("/personas", persona("own")).await,
        e.admin.make("/personas", persona("o")).await,
    );
    let f = e
        .chat
        .make("/folders", json!({"name": "f", "persona_id": other["id"]}))
        .await;
    let s = session(
        &e,
        json!({"name": "s", "folder_id": f["id"], "persona_id": own["id"]}),
    )
    .await;
    assert_eq!(
        (s["persona_id"].clone(), s["effective_persona_id"].clone()),
        (own["id"].clone(), own["id"].clone())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_effective_from_folder() {
    let e = Env::start().await;
    let outer = e.admin.make("/personas", persona("outer")).await;
    let inner = e.admin.make("/personas", persona("inner")).await;
    let a = e
        .chat
        .make("/folders", json!({"name": "a", "persona_id": outer["id"]}))
        .await;
    let b = e
        .chat
        .make("/folders", json!({"name": "b", "parent_id": a["id"]}))
        .await;
    let c = e
        .chat
        .make(
            "/folders",
            json!({"name": "c", "parent_id": b["id"], "persona_id": inner["id"]}),
        )
        .await;
    let d = e
        .chat
        .make("/folders", json!({"name": "d", "parent_id": c["id"]}))
        .await;
    let in_b = session(&e, json!({"name": "s1", "folder_id": b["id"]})).await;
    let in_d = session(&e, json!({"name": "s2", "folder_id": d["id"]})).await;
    assert_eq!(in_b["persona_id"], Value::Null, "own is not set");
    assert_eq!(
        in_b["effective_persona_id"], outer["id"],
        "nearest ancestor: a"
    );
    assert_eq!(
        in_d["effective_persona_id"], inner["id"],
        "nearest ancestor: c"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_effective_default() {
    let e = Env::start().await;
    let f = e.chat.make("/folders", json!({"name": "a"})).await;
    let s = session(&e, json!({"name": "s", "folder_id": f["id"]})).await;
    assert_eq!(s["effective_persona_id"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_not_copied_on_create() {
    let e = Env::start().await;
    let (p1, p2) = (
        e.admin.make("/personas", persona("1")).await,
        e.admin.make("/personas", persona("2")).await,
    );
    let f = e
        .chat
        .make("/folders", json!({"name": "a", "persona_id": p1["id"]}))
        .await;
    let s = session(&e, json!({"name": "s", "folder_id": f["id"]})).await;
    assert_eq!(s["effective_persona_id"], p1["id"]);
    e.chat
        .patch(
            &format!("/folders/{}", f["id"]),
            json!({"persona_id": p2["id"]}),
        )
        .await;
    let after = get(&e, &s["id"]).await;
    assert_eq!(
        after["persona_id"],
        Value::Null,
        "the persona is not copied into the session"
    );
    assert_eq!(
        after["effective_persona_id"], p2["id"],
        "effective persona changed together with the folder"
    );
    // Moving a session to a folder without a persona changes the effective one.
    let g = e.chat.make("/folders", json!({"name": "g"})).await;
    e.chat
        .patch(
            &format!("/sessions/{}", s["id"]),
            json!({"folder_id": g["id"]}),
        )
        .await;
    assert_eq!(get(&e, &s["id"]).await["effective_persona_id"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_inherits_persona_path_ctx() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", persona("p")).await;
    let path = e.home.join("proj").to_string_lossy().into_owned();
    let parent = session(
        &e,
        json!({"name": "parent", "persona_id": p["id"], "path": path}),
    )
    .await;
    let (st, t) = e
        .chat
        .post(
            &format!("/sessions/{}/threads", parent["id"]),
            json!({"name": "thread"}),
        )
        .await;
    assert_eq!(st, 201, "{t}");
    assert_eq!(t["parent_id"], parent["id"]);
    assert_eq!(t["persona_id"], Value::Null, "no own value");
    assert_eq!(t["effective_persona_id"], p["id"], "parent persona");
    assert_eq!(t["path"].as_str(), Some(path.as_str()), "parent path");
    assert_eq!(t["cfg"]["ctx"], "parent", "parent context by default");
    // Disabling the context.
    let (st, t2) = e
        .chat
        .patch(
            &format!("/sessions/{}", t["id"]),
            json!({"cfg": {"ctx": "none"}}),
        )
        .await;
    assert_eq!((st, t2["cfg"]["ctx"].clone()), (200, json!("none")), "{t2}");
    // A change of the parent persona and path is visible in the thread.
    let q = e.admin.make("/personas", persona("q")).await;
    e.chat
        .patch(
            &format!("/sessions/{}", parent["id"]),
            json!({"persona_id": q["id"], "path": null}),
        )
        .await;
    let t3 = get(&e, &t["id"]).await;
    assert_eq!(
        (t3["effective_persona_id"].clone(), t3["path"].clone()),
        (q["id"].clone(), Value::Null)
    );
    // A thread inside a thread, or moving a thread apart from its parent, is not allowed.
    let (st, v) = e
        .chat
        .post(
            &format!("/sessions/{}/threads", t["id"]),
            json!({"name": "x"}),
        )
        .await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (422, Some("validation_failed")),
        "{v}"
    );
    let f = e.chat.make("/folders", json!({"name": "f"})).await;
    let (st, v) = e
        .chat
        .patch(
            &format!("/sessions/{}", t["id"]),
            json!({"folder_id": f["id"]}),
        )
        .await;
    assert_eq!((st, fields(&v)), (422, vec!["folder_id".to_string()]));
    assert_eq!(
        e.chat
            .post("/sessions/999/threads", json!({"name": "x"}))
            .await
            .0,
        404
    );
}
