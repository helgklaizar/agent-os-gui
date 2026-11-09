//! A5-11 (DB): `TaskRepo` and `TriggerRepo` on a real DB: selection of due items, `next_run` shift, run history,
//! closing interrupted runs, cascades, and disabling triggers when a session is deleted.

use std::sync::Arc;

use agentd_core::{
    NewTask, SystemClock, TaskError, TaskFilter, TaskStore, Trigger, TriggerError, TriggerStore,
};
use agentd_db::{Db, DbConfig, TaskRepo, TriggerRepo, params};
use serde_json::json;

struct Rig {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    tasks: TaskRepo,
    triggers: TriggerRepo,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("agentos/sessions.db"))).unwrap());
    Rig {
        tasks: TaskRepo::new(db.clone(), Arc::new(SystemClock)),
        triggers: TriggerRepo::new(db.clone()),
        db,
        _dir: dir,
    }
}

async fn session(r: &Rig) -> i64 {
    r.db.write(|tx| {
        tx.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
            [],
        )?;
        Ok(tx.last_insert_rowid())
    })
    .await
    .unwrap()
}

fn new_task(session_id: i64, next_run: Option<i64>) -> NewTask {
    NewTask {
        kind: "sched".into(),
        title: "t".into(),
        spec: "s".into(),
        cron: Some("0 8 * * *".into()),
        tz: "Asia/Jerusalem".into(),
        persona_id: None,
        session_id,
        next_run,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn task_repo_due_set_next_and_run_history() {
    let r = rig().await;
    let s = session(&r).await;
    let t = r.tasks.create(new_task(s, Some(100))).await.unwrap();
    assert!(r.tasks.due(99).await.unwrap().is_empty(), "too early");
    assert_eq!(r.tasks.due(100).await.unwrap().len(), 1, "due");
    r.tasks.set_next(t.id, Some(500), None).await.unwrap();
    assert!(
        r.tasks.due(100).await.unwrap().is_empty(),
        "next_run shifted"
    );
    r.tasks.set_next(t.id, None, Some("paused")).await.unwrap();
    assert!(
        r.tasks.due(1_000_000).await.unwrap().is_empty(),
        "paused is not selected"
    );

    let mut ids = Vec::new();
    for i in 0..3 {
        let run = r.tasks.start_run(t.id, 10 + i, "running").await.unwrap();
        ids.push(run.id);
    }
    r.tasks.finish_run(ids[0], 20, "ok", None).await.unwrap();
    let page = r.tasks.runs(t.id, None, 2).await.unwrap();
    assert_eq!(
        page.iter().map(|x| x.id).collect::<Vec<_>>(),
        [ids[2], ids[1]],
        "newest to oldest"
    );
    let next = r.tasks.runs(t.id, Some(ids[1]), 10).await.unwrap();
    assert_eq!(
        next.iter().map(|x| x.id).collect::<Vec<_>>(),
        [ids[0]],
        "before by id"
    );
    assert_eq!(next[0].status, "ok");

    let n = r.tasks.fail_stale_runs(99).await.unwrap();
    assert_eq!(n, 2, "two unfinished runs closed after a crash");
    let all = r.tasks.runs(t.id, None, 10).await.unwrap();
    assert!(
        all.iter()
            .filter(|x| x.id != ids[0])
            .all(|x| x.status == "error"
                && x.output.as_deref() == Some("core_restart")
                && x.finished_at == Some(99))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_repo_rejects_missing_session_and_persona_and_cascades() {
    let r = rig().await;
    assert_eq!(
        r.tasks.create(new_task(999, None)).await.unwrap_err(),
        TaskError::SessionNotFound
    );
    let s = session(&r).await;
    let mut bad = new_task(s, None);
    bad.persona_id = Some(777);
    assert!(matches!(
        r.tasks.create(bad).await,
        Err(TaskError::Invalid(f)) if f[0].field == "persona_id"
    ));
    let t = r.tasks.create(new_task(s, None)).await.unwrap();
    r.tasks.start_run(t.id, 1, "running").await.unwrap();
    r.tasks.delete(t.id).await.unwrap();
    assert_eq!(
        r.tasks.runs(t.id, None, 10).await.unwrap_err(),
        TaskError::NotFound
    );
    let left: i64 =
        r.db.read(|c| c.query_row("SELECT count(*) FROM task_runs", [], |x| x.get(0)))
            .await
            .unwrap();
    assert_eq!(left, 0, "run history went away with the task");
    assert_eq!(r.tasks.delete(t.id).await.unwrap_err(), TaskError::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_repo_filter_and_session_delete_pauses() {
    let r = rig().await;
    let (a, b) = (session(&r).await, session(&r).await);
    let ta = r.tasks.create(new_task(a, Some(1))).await.unwrap();
    r.tasks.create(new_task(b, Some(1))).await.unwrap();
    let only_a = r
        .tasks
        .list(TaskFilter {
            session_id: Some(a),
            ..TaskFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(only_a.len(), 1);
    r.db.write(move |tx| tx.execute("DELETE FROM sessions WHERE id = ?1", params![a]))
        .await
        .unwrap();
    let got = r.tasks.get(ta.id).await.unwrap();
    assert_eq!((got.state.as_str(), got.session_id), ("paused", None));
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_repo_roundtrip_and_session_delete_disables() {
    let r = rig().await;
    let s = session(&r).await;
    let t = Trigger {
        id: 0,
        source: "tg".into(),
        match_: json!({"from": "@a"}),
        session_id: Some(s),
        task_id: None,
        enabled: true,
    };
    assert_eq!(
        r.triggers
            .create(Trigger {
                session_id: Some(404),
                ..t.clone()
            })
            .await
            .unwrap_err(),
        TriggerError::SessionNotFound
    );
    let made = r.triggers.create(t.clone()).await.unwrap();
    assert_eq!(made.match_, json!({"from": "@a"}));
    assert_eq!(made.task_id, None);
    // The "create run" action is stored next to the conditions and does not enter `match`.
    let with_task = r
        .triggers
        .create(Trigger {
            task_id: Some(7),
            ..t
        })
        .await
        .unwrap();
    assert_eq!(with_task.task_id, Some(7));
    assert_eq!(with_task.match_, json!({"from": "@a"}));
    assert_eq!(r.triggers.get(with_task.id).await.unwrap().task_id, Some(7));
    r.db.write(move |tx| tx.execute("DELETE FROM sessions WHERE id = ?1", params![s]))
        .await
        .unwrap();
    let got = r.triggers.get(made.id).await.unwrap();
    assert_eq!(
        (got.enabled, got.session_id),
        (false, None),
        "trigger disabled and has no session"
    );
    r.triggers.delete(made.id).await.unwrap();
    assert_eq!(
        r.triggers.delete(made.id).await.unwrap_err(),
        TriggerError::NotFound
    );
}
