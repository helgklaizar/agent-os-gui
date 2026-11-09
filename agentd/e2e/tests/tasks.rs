//! A5-09: tasks — routes, body validation, pause and resume, frames in the session topic, deletion.

mod common;

use std::time::Duration;

use common::{Env, chrono_now, deleg, fields, finished_runs, mk_task, sched};
use core_client::api::FrameKind;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn task_create_list_and_next_run() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, sched("#scn:chunks hello")).await;
    assert_eq!(
        (t["kind"].as_str(), t["state"].as_str(), t["tz"].as_str()),
        (Some("sched"), Some("active"), Some("Asia/Jerusalem")),
        "{t}"
    );
    assert_eq!(t["session_id"], s);
    let next = t["next_run"].as_i64().unwrap();
    let now = chrono_now();
    assert!(next > now && next - now <= 86_400, "{next} versus {now}");
    let (_, list) = e.read.get(&format!("/sessions/{s}/tasks")).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, all) = e.read.get("/tasks?kind=sched&state=active").await;
    assert_eq!(all.as_array().unwrap()[0]["id"], t["id"]);
    let (_, none) = e.read.get("/tasks?kind=deleg").await;
    assert!(none.as_array().unwrap().is_empty());
    let d = mk_task(&e, s, deleg("#scn:chunks")).await;
    assert_eq!(d["next_run"], Value::Null, "delegation — no schedule");
}

#[tokio::test(flavor = "multi_thread")]
async fn task_validation_errors() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let cases = [
        (json!({"kind": "x", "title": "a", "spec": "b"}), "kind"),
        (
            json!({"kind": "sched", "title": "", "spec": "b", "cron": "0 8 * * *"}),
            "title",
        ),
        (
            json!({"kind": "sched", "title": "a", "spec": " ", "cron": "0 8 * * *"}),
            "spec",
        ),
        (json!({"kind": "sched", "title": "a", "spec": "b"}), "cron"),
        (
            json!({"kind": "sched", "title": "a", "spec": "b", "cron": "0 0 8 * * *"}),
            "cron",
        ),
        (
            json!({"kind": "sched", "title": "a", "spec": "b", "cron": "0 8 * * *", "tz": "Mars/Base"}),
            "tz",
        ),
        (
            json!({"kind": "deleg", "title": "a", "spec": "b", "cron": "0 8 * * *"}),
            "cron",
        ),
    ];
    for (body, field) in cases {
        let (st, v) = e
            .chat
            .post(&format!("/sessions/{s}/tasks"), body.clone())
            .await;
        assert_eq!(st, 422, "{body}: {v}");
        assert!(fields(&v).contains(&field.to_string()), "{field}: {v}");
    }
    let (st, _) = e.chat.post("/sessions/9999/tasks", sched("x")).await;
    assert_eq!(st, 404);
    let (st, _) = e
        .read
        .post(&format!("/sessions/{s}/tasks"), sched("x"))
        .await;
    assert_eq!(st, 403, "a read token does not create tasks");
}

#[tokio::test(flavor = "multi_thread")]
async fn task_pause_resume() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, sched("#scn:chunks")).await;
    let id = t["id"].as_i64().unwrap();
    let (st, p) = e
        .chat
        .patch(&format!("/tasks/{id}"), json!({"state": "paused"}))
        .await;
    assert_eq!(st, 200, "{p}");
    assert_eq!(
        (p["state"].as_str(), &p["next_run"]),
        (Some("paused"), &Value::Null)
    );
    let (st, r) = e
        .chat
        .patch(&format!("/tasks/{id}"), json!({"state": "active"}))
        .await;
    assert_eq!(st, 200, "{r}");
    assert!(
        r["next_run"].as_i64().unwrap() > chrono_now(),
        "next_run recomputed"
    );
    let (st, c) = e
        .chat
        .patch(
            &format!("/tasks/{id}"),
            json!({"cron": "30 9 * * *", "title": "later"}),
        )
        .await;
    assert_eq!((st, c["title"].as_str()), (200, Some("later")), "{c}");
    assert_ne!(
        c["next_run"], r["next_run"],
        "changing cron recomputes next_run"
    );
    let (st, v) = e
        .chat
        .patch(&format!("/tasks/{id}"), json!({"cron": "bad"}))
        .await;
    assert_eq!(st, 422, "{v}");
    let (st, _) = e.chat.patch("/tasks/9999", json!({"title": "x"})).await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_frames_in_session_topic() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let mut ws = e.ws();
    let snap = ws.subscribe(&format!("session.{s}"));
    assert_eq!(
        snap["tasks"],
        json!([]),
        "the session snapshot has the task list"
    );
    let t = mk_task(&e, s, deleg("#scn:chunks")).await;
    let id = t["id"].as_i64().unwrap();
    let f = ws.until(Duration::from_secs(10), |f| {
        f.data["kind"] == "task.created"
    });
    assert_eq!(f.last().unwrap().data["task"]["id"], id);
    e.chat
        .patch(&format!("/tasks/{id}"), json!({"title": "new name"}))
        .await;
    let f = ws.until(Duration::from_secs(10), |f| {
        f.data["kind"] == "task.updated"
    });
    assert_eq!(f.last().unwrap().data["task"]["title"], "new name");
    e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    let f = ws.until(Duration::from_secs(15), |f| {
        f.data["kind"] == "task.run" && f.data["run"]["status"] == "ok"
    });
    assert!(
        f.iter().any(|x| x.data["run"]["status"] == "running"),
        "start and end of the run"
    );
    assert_eq!(f.last().unwrap().kind, FrameKind::Event);
    let snap = e.read.get(&format!("/sessions/{s}/tasks")).await.1;
    assert_eq!(snap.as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_session_delete_pauses() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, sched("#scn:chunks")).await;
    let id = t["id"].as_i64().unwrap();
    assert_eq!(e.chat.delete(&format!("/sessions/{s}")).await.0, 204);
    let (_, all) = e.read.get("/tasks").await;
    let got = all
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap();
    assert_eq!(
        (got["state"].as_str(), &got["session_id"]),
        (Some("paused"), &Value::Null),
        "{got}"
    );
    let (st, v) = e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    assert_eq!(st, 409, "nothing to run without a session: {v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn task_delete_removes_history() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, deleg("#scn:chunks")).await;
    let id = t["id"].as_i64().unwrap();
    e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    finished_runs(&e, id, 1).await;
    assert_eq!(e.chat.delete(&format!("/tasks/{id}")).await.0, 204);
    assert_eq!(e.read.get(&format!("/tasks/{id}/runs")).await.0, 404);
    assert_eq!(e.chat.delete(&format!("/tasks/{id}")).await.0, 404);
}
