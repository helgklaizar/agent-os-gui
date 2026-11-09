//! Task runs: a tick by `next_run`, manual runs, a queue with a concurrency limit, execution as a request to the session
//! on behalf of `cron`. Runs missed during idle time are recorded as `skipped` and not made up.

use std::time::Duration;

use super::service::{MISSED_GRACE_SECS, Tasks};
use super::{Task, TaskError, TaskFilter, TaskResult, TaskRun, cron_next};
use crate::{ChatError, EventLevel, RunAs, UserMessage};

/// A run waits for the session reply no longer than this (like the overall reply limit, with headroom for the queue).
const RUN_WAIT: Duration = Duration::from_secs(660);

impl Tasks {
    fn publish_run(&self, task: &Task, run: &TaskRun) {
        self.frame(task, "task.run", ("run", run.to_json()));
    }

    /// Run now (`POST /tasks/{id}/run`): a `running` record, a frame, an event, background execution.
    pub async fn run_now(&self, id: i64) -> TaskResult<TaskRun> {
        let task = self.inner.store.get(id).await?;
        if task.session_id.is_none() {
            return Err(TaskError::NoSession);
        }
        self.begin(task).await
    }

    async fn begin(&self, task: Task) -> TaskResult<TaskRun> {
        let now = self.inner.clock.now_unix();
        let run = self.inner.store.start_run(task.id, now, "running").await?;
        self.publish_run(&task, &run);
        self.emit(
            &task,
            &task.kind,
            EventLevel::Info,
            format!("Run: {}", task.title),
        )
        .await;
        let me = self.clone();
        let started = run.clone();
        tokio::spawn(async move { me.execute(task, started).await });
        Ok(run)
    }

    async fn acquire(&self) {
        loop {
            let limit = self.max_parallel().await;
            {
                let mut n = self
                    .inner
                    .running
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if *n < limit {
                    *n += 1;
                    return;
                }
            }
            tokio::select! {
                () = self.inner.slot.notified() => {}
                () = tokio::time::sleep(Duration::from_millis(300)) => {}
            }
        }
    }

    fn release(&self) {
        let mut n = self
            .inner
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *n = n.saturating_sub(1);
        drop(n);
        self.inner.slot.notify_waiters();
    }

    /// Session reply to the task `spec`: the run outcome (status; on success the start of the agent reply, on error the reason).
    async fn drive(&self, task: &Task, run: &TaskRun) -> (&'static str, Option<String>) {
        let Some(sid) = task.session_id else {
            return ("error", Some("no_session".into()));
        };
        let msg = UserMessage {
            client_id: format!("task-run-{}", run.id),
            body_md: task.spec.clone(),
            attachment_ids: Vec::new(),
        };
        let rx = match self.inner.chat.send_as(sid, msg, RunAs::cron()).await {
            Ok((_, Some(rx))) => rx,
            Ok((_, None)) => return ("ok", None),
            Err(ChatError::QueueFull) => return ("error", Some("queue_full".into())),
            Err(ChatError::NotFound) => return ("error", Some("no_session".into())),
            Err(_) => return ("error", Some("chat_error".into())),
        };
        match tokio::time::timeout(RUN_WAIT, rx).await {
            Ok(Ok(end)) if end.expired => ("error", Some("expired".into())),
            Ok(Ok(end)) if end.status == "done" => ("ok", Some(end.text).filter(|t| !t.is_empty())),
            Ok(Ok(end)) => ("error", Some(end.status.to_string())),
            _ => ("error", Some("timeout".into())),
        }
    }

    async fn execute(&self, task: Task, run: TaskRun) {
        self.acquire().await;
        let (status, output) = self.drive(&task, &run).await;
        self.release();
        let now = self.inner.clock.now_unix();
        let finished = self
            .inner
            .store
            .finish_run(run.id, now, status, output.clone())
            .await;
        let Ok(done) = finished else {
            tracing::warn!(
                scope = "tasks",
                event = "finish_failed",
                "run outcome not recorded"
            );
            return;
        };
        self.publish_run(&task, &done);
        let (level, text) = if status == "ok" {
            (EventLevel::Ok, format!("Done: {}", task.title))
        } else {
            (
                EventLevel::Error,
                format!("Error: {} ({})", task.title, output.unwrap_or_default()),
            )
        };
        self.emit(&task, "done", level, text).await;
        if status == "ok"
            && task.kind == "deleg"
            && let Ok(t) = self.inner.store.set_next(task.id, None, Some("done")).await
        {
            self.frame(&t, "task.updated", ("task", t.to_json()));
        }
    }

    async fn skip(&self, task: &Task, why: &str) {
        let now = self.inner.clock.now_unix();
        let Ok(run) = self.inner.store.start_run(task.id, now, "skipped").await else {
            return;
        };
        if let Ok(done) = self
            .inner
            .store
            .finish_run(run.id, now, "skipped", Some(why.to_string()))
            .await
        {
            self.publish_run(task, &done);
        }
        self.emit(
            task,
            &task.kind,
            EventLevel::Warn,
            format!("Run skipped: {}", task.title),
        )
        .await;
    }

    /// One scheduler pass: tasks with `next_run <= now`. `next_run` is advanced before the run (exactly one run per
    /// firing); lateness over `MISSED_GRACE_SECS` gives `skipped`, and missed runs are not made up.
    pub async fn tick(&self) {
        let now = self.inner.clock.now_unix();
        let Ok(due) = self.inner.store.due(now).await else {
            return;
        };
        for t in due {
            let next = t
                .cron
                .as_deref()
                .and_then(|c| cron_next(c, &t.tz, now).ok().flatten());
            if self.inner.store.set_next(t.id, next, None).await.is_err() {
                continue;
            }
            let late = now - t.next_run.unwrap_or(now);
            if late > MISSED_GRACE_SECS {
                self.skip(&t, "missed").await;
            } else if t.session_id.is_none() {
                self.skip(&t, "no_session").await;
            } else if self.begin(t).await.is_err() {
                tracing::warn!(scope = "tasks", event = "begin_failed", "run not started");
            }
        }
    }

    /// All tasks of a session (`GET /sessions/{id}/tasks`).
    pub async fn of_session(&self, session_id: i64) -> TaskResult<Vec<Task>> {
        self.list(TaskFilter {
            session_id: Some(session_id),
            ..TaskFilter::default()
        })
        .await
    }
}
