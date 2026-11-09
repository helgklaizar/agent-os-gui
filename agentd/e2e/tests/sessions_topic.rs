//! A2-07: `sessions` topic (`snapshot`, exactly one `delta` per mutation) and unread counters.

mod common;

use std::time::Duration;

use common::{Env, persona};
use serde_json::{Value, json};

const WINDOW: Duration = Duration::from_millis(400);

#[tokio::test(flavor = "multi_thread")]
async fn sessions_topic_snapshot_and_delta() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", persona("p")).await;
    let pre = e
        .chat
        .make("/folders", json!({"name": "before subscribe"}))
        .await;
    let pre_s = e
        .chat
        .make(
            "/sessions",
            json!({"name": "before", "folder_id": pre["id"]}),
        )
        .await;
    let pre_t = e
        .chat
        .make(
            &format!("/sessions/{}/threads", pre_s["id"]),
            json!({"name": "thread"}),
        )
        .await;
    let mut ws = e.ws();
    let snap = ws.subscribe("sessions");
    assert_eq!(snap["folders"].as_array().unwrap().len(), 1, "{snap}");
    let ids: Vec<&Value> = snap["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["id"])
        .collect();
    assert_eq!(
        ids,
        [&pre_s["id"], &pre_t["id"]],
        "the snapshot has both sessions and threads"
    );
    assert_eq!(snap["sessions"][0]["effective_persona_id"], Value::Null);

    let mut seen: Vec<(String, Value)> = Vec::new();
    let mut step = |ws: &mut common::Ws, what: &str| {
        let frames = ws.drain(WINDOW);
        assert_eq!(frames.len(), 1, "{what}: exactly one delta, got {frames:?}");
        assert_eq!(frames[0].topic, "sessions");
        seen.push((
            frames[0].data["op"].as_str().unwrap().to_string(),
            frames[0].data.clone(),
        ));
        frames[0].seq
    };
    let f = e
        .chat
        .make("/folders", json!({"name": "new", "persona_id": p["id"]}))
        .await;
    let s1 = step(&mut ws, "folder created");
    e.chat
        .patch(&format!("/folders/{}", f["id"]), json!({"open": false}))
        .await;
    let s2 = step(&mut ws, "folder changed");
    let s = e
        .chat
        .make("/sessions", json!({"name": "s", "folder_id": f["id"]}))
        .await;
    let s3 = step(&mut ws, "session created");
    e.chat
        .patch(
            &format!("/sessions/{}", s["id"]),
            json!({"folder_id": null}),
        )
        .await;
    let s4 = step(&mut ws, "move");
    e.chat
        .make(
            &format!("/sessions/{}/threads", s["id"]),
            json!({"name": "t"}),
        )
        .await;
    step(&mut ws, "thread created");
    e.chat.delete(&format!("/sessions/{}", s["id"])).await;
    // Deleting a session with a thread: one delta for the deleted session (the client also drops threads by parent_id).
    step(&mut ws, "session deleted");
    e.chat.delete(&format!("/folders/{}", f["id"])).await;
    step(&mut ws, "folder deleted");
    let ops: Vec<&str> = seen.iter().map(|(o, _)| o.as_str()).collect();
    assert_eq!(
        ops,
        [
            "folder.created",
            "folder.updated",
            "session.created",
            "session.updated",
            "session.created",
            "session.deleted",
            "folder.deleted"
        ]
    );
    assert_eq!(seen[0].1["folder"]["name"], "new");
    assert_eq!(seen[0].1["folder"]["persona_id"], p["id"]);
    assert_eq!(seen[3].1["session"]["folder_id"], Value::Null);
    assert_eq!(seen[3].1["session"]["effective_persona_id"], Value::Null);
    assert_eq!(
        seen[5].1["session"],
        json!({"id": s["id"]}),
        "on delete only the id"
    );
    assert!(
        s1 < s2 && s2 < s3 && s3 < s4,
        "seq grows without gaps: {s1} {s2} {s3} {s4}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_delta_not_sent_on_rollback() {
    let e = Env::start().await;
    let a = e.chat.make("/folders", json!({"name": "a"})).await;
    let b = e
        .chat
        .make("/folders", json!({"name": "b", "parent_id": a["id"]}))
        .await;
    let mut ws = e.ws();
    ws.subscribe("sessions");
    // The cycle trigger rolls back the whole PATCH.
    let (st, _) = e
        .chat
        .patch(
            &format!("/folders/{}", a["id"]),
            json!({"name": "x", "parent_id": b["id"]}),
        )
        .await;
    assert_eq!(st, 409);
    // Checks before writing: 422 and 404 also produce no deltas.
    assert_eq!(
        e.chat
            .patch(&format!("/folders/{}", a["id"]), json!({"persona_id": 999}))
            .await
            .0,
        422
    );
    assert_eq!(e.chat.post("/sessions", json!({"name": ""})).await.0, 422);
    assert_eq!(e.chat.delete("/sessions/999").await.0, 404);
    assert_eq!(ws.drain(WINDOW).len(), 0, "no deltas at all");
    let (_, list) = e.read.get("/folders").await;
    assert_eq!(list[0]["name"], "a");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_read_resets_unread() {
    let e = Env::start_with(|c| {
        c.execute_batch(
            "INSERT INTO sessions(name, unread, created_at, updated_at) VALUES ('s', 3, 1, 1);",
        )
    })
    .await;
    assert_eq!(e.read.get("/sessions/1").await.1["unread"], 3);
    let mut ws = e.ws();
    assert_eq!(ws.subscribe("sessions")["sessions"][0]["unread"], 3);
    let (st, body) = e.chat.post("/sessions/1/read", json!({})).await;
    assert_eq!((st, body), (204, Value::Null));
    assert_eq!(e.read.get("/sessions/1").await.1["unread"], 0);
    let frames = ws.drain(WINDOW);
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(
        frames[0].data,
        json!({"op": "unread", "session_id": 1, "unread": 0})
    );
    // Repeat: nothing changed, so no delta.
    assert_eq!(e.chat.post("/sessions/1/read", json!({})).await.0, 204);
    assert_eq!(ws.drain(WINDOW).len(), 0);
    assert_eq!(e.chat.post("/sessions/999/read", json!({})).await.0, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_topic_empty_snapshot() {
    let e = Env::start().await;
    let mut ws = e.ws();
    let snap = ws.subscribe("sessions");
    assert_eq!(snap, json!({"folders": [], "sessions": []}));
}
