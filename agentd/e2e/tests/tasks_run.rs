//! A5-09: running tasks on a schedule and manually, skipping when idle, the concurrency limit, actions on behalf of `cron`.
//! Time is the core's controllable clock (`Env::start_clock`).

mod common;

use std::time::Duration;

use common::{Env, deleg, finished_runs, mk_task, runs, sched, task_events};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn task_run_now_202() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, deleg("#scn:chunks do it")).await;
    let id = t["id"].as_i64().unwrap();
    let (st, run) = e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    assert_eq!(st, 202, "{run}");
    assert_eq!(
        (run["task_id"].as_i64(), run["status"].as_str()),
        (Some(id), Some("running"))
    );
    let done = finished_runs(&e, id, 1).await;
    assert_eq!(done[0]["status"], "ok", "{done:?}");
    assert_eq!(done[0]["id"], run["id"]);
    let msgs = e.messages(s).await;
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "user" && m["body_md"] == "#scn:chunks do it"),
        "spec went to the session as a message"
    );
    assert_eq!(task_events(&e, "deleg").await.len(), 1, "event task.deleg");
    let finished = task_events(&e, "done").await;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["level"], "ok");
    let (_, after) = e.read.get(&format!("/sessions/{s}/tasks")).await;
    assert_eq!(after[0]["state"], "done", "assignment completed");
    let (st, _) = e.read.post(&format!("/tasks/{id}/run"), json!({})).await;
    assert_eq!(st, 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_due_run_on_schedule_executes_and_reschedules() {
    let e = Env::start_clock().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, sched("#scn:chunks on schedule")).await;
    let id = t["id"].as_i64().unwrap();
    let next = t["next_run"].as_i64().unwrap();
    // The shift is computed from the core clock (`created_at` was taken by it too), not the real one: under load they diverge.
    e.advance(next - t["created_at"].as_i64().unwrap() + 1)
        .await;
    let done = finished_runs(&e, id, 1).await;
    assert_eq!(done[0]["status"], "ok", "{done:?}");
    let (_, after) = e.read.get(&format!("/sessions/{s}/tasks")).await;
    assert_eq!(after[0]["state"], "active");
    assert!(
        after[0]["next_run"].as_i64().unwrap() >= next + 86_400 - 1,
        "{after}"
    );
    assert_eq!(task_events(&e, "sched").await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_missed_run_skipped() {
    let e = Env::start_clock().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(
        &e,
        s,
        json!({"kind": "sched", "title": "often", "spec": "#scn:chunks", "cron": "*/5 * * * *"}),
    )
    .await;
    let id = t["id"].as_i64().unwrap();
    // The core was "idle" for a day: missed runs are not caught up, one `skipped` is recorded.
    e.advance(86_400).await;
    let done = finished_runs(&e, id, 1).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let all = runs(&e, id).await;
    assert_eq!(all.len(), 1, "not 288 runs: {all:?}");
    assert_eq!(done[0]["status"], "skipped");
    let (_, after) = e.read.get(&format!("/sessions/{}/tasks", s)).await;
    assert!(after[0]["next_run"].as_i64().unwrap() > t["next_run"].as_i64().unwrap() + 80_000);
    let msgs = e.messages(s).await;
    assert!(
        msgs.iter().all(|m| m["role"] != "user"),
        "no message went to the session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_max_parallel_respected() {
    let e = Env::start().await;
    let (_, page) = e.admin.get("/settings/personas").await;
    let rev = page["rev"].as_i64().unwrap_or(0);
    let (st, v) = e
        .admin
        .patch_if_match(
            "/settings/personas",
            json!({"balance": {"max_parallel": 1}}),
            rev,
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (_, s) = e.model_session().await;
        let t = mk_task(&e, s, deleg("#scn:slow wait")).await;
        ids.push(t["id"].as_i64().unwrap());
    }
    for id in &ids {
        let (st, _) = e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
        assert_eq!(st, 202);
    }
    let mut ends = Vec::new();
    for id in &ids {
        ends.push(
            finished_runs(&e, *id, 1).await[0]["finished_at"]
                .as_i64()
                .unwrap(),
        );
    }
    ends.sort_unstable();
    assert!(
        ends[2] - ends[0] >= 4,
        "three runs of 2.5 s go one at a time: {ends:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_cron_actor_in_action_log() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, deleg("#scn:action_read read")).await;
    let id = t["id"].as_i64().unwrap();
    e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    finished_runs(&e, id, 1).await;
    let actors: Vec<String> = e
        .inspect(|c| {
            c.prepare("SELECT actor FROM actions_log WHERE action = 'read'")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(actors, ["cron"], "the task action runs on behalf of cron");
}

#[tokio::test(flavor = "multi_thread")]
async fn task_action_ask_expires_denied() {
    let e = Env::start_clock().await;
    let (_, s) = e.model_session().await;
    let t = mk_task(&e, s, deleg("#scn:action_send write")).await;
    let id = t["id"].as_i64().unwrap();
    e.chat.post(&format!("/tasks/{id}/run"), json!({})).await;
    let (_, card) = e.wait_card(s).await;
    assert_eq!(card["meta"]["request"]["action"], "send");
    e.advance(601).await;
    let done = finished_runs(&e, id, 1).await;
    assert_eq!(done[0]["status"], "error", "{done:?}");
    assert!(
        done[0]["output"].as_str().unwrap().contains("expired"),
        "{done:?}"
    );
    let ev = task_events(&e, "done").await;
    assert_eq!(ev[0]["level"], "error", "{ev:?}");
}
