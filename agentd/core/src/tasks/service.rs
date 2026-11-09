//! Task service: creation and edits with `next_run` calculation, frames in `session.<id>`, the scheduler tick.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{
    NewTask, Task, TaskError, TaskFilter, TaskResult, TaskRun, TaskStore, TaskUpdate, cron_check,
    cron_next, parse_create, parse_edit,
};
use crate::{
    ChatService, Clock, EventLevel, Events, FieldError, NewEvent, Notifier, SettingsStore,
};

/// How many tasks run at once when `balance.max_parallel` is not set.
pub const DEFAULT_MAX_PARALLEL: usize = 2;
/// A run that is late by more than this many seconds (the core was idle) is skipped and not made up.
pub const MISSED_GRACE_SECS: i64 = 120;
const MAX_PARALLEL_LIMIT: usize = 16;

pub(super) struct Inner {
    pub store: Arc<dyn TaskStore>,
    pub clock: Arc<dyn Clock>,
    pub notifier: Arc<dyn Notifier>,
    pub events: Events,
    pub settings: Arc<dyn SettingsStore>,
    pub chat: ChatService,
    pub running: Mutex<usize>,
    pub slot: Notify,
}

#[derive(Clone)]
pub struct Tasks {
    pub(super) inner: Arc<Inner>,
}

fn topic(session_id: i64) -> String {
    format!("session.{session_id}")
}

/// `next_run` by state: only an active `sched` with a schedule.
fn compute_next(kind: &str, cron: Option<&str>, tz: &str, state: &str, now: i64) -> Option<i64> {
    if kind != "sched" || state != "active" {
        return None;
    }
    cron.and_then(|c| cron_next(c, tz, now).ok().flatten())
}

impl Tasks {
    #[must_use]
    pub fn new(
        store: Arc<dyn TaskStore>,
        clock: Arc<dyn Clock>,
        notifier: Arc<dyn Notifier>,
        events: Events,
        settings: Arc<dyn SettingsStore>,
        chat: ChatService,
    ) -> Tasks {
        Tasks {
            inner: Arc::new(Inner {
                store,
                clock,
                notifier,
                events,
                settings,
                chat,
                running: Mutex::new(0),
                slot: Notify::new(),
            }),
        }
    }

    #[must_use]
    pub fn store(&self) -> &Arc<dyn TaskStore> {
        &self.inner.store
    }

    pub(super) fn frame(&self, task: &Task, kind: &str, data: (&str, Value)) {
        if let Some(sid) = task.session_id {
            self.inner
                .notifier
                .publish_event(&topic(sid), json!({"kind": kind, data.0: data.1}));
        }
    }

    pub(super) async fn emit(&self, task: &Task, sub: &str, level: EventLevel, text: String) {
        let ev = NewEvent::new("task", sub, level, text).session(task.session_id);
        self.inner.events.emit(ev).await;
    }

    /// Parallel run limit: `personas.balance.max_parallel`.
    pub(super) async fn max_parallel(&self) -> usize {
        let n = self
            .inner
            .settings
            .get("personas")
            .await
            .ok()
            .and_then(|d| d.doc["balance"]["max_parallel"].as_u64())
            .and_then(|n| usize::try_from(n).ok());
        n.map_or(DEFAULT_MAX_PARALLEL, |n| n.clamp(1, MAX_PARALLEL_LIMIT))
    }

    /// `POST /sessions/{id}/tasks`.
    pub async fn create(&self, session_id: i64, body: &Value) -> TaskResult<Task> {
        let c = parse_create(body).map_err(TaskError::Invalid)?;
        let now = self.inner.clock.now_unix();
        let next_run = compute_next(&c.kind, c.cron.as_deref(), &c.tz, "active", now);
        let task = self
            .inner
            .store
            .create(NewTask {
                kind: c.kind,
                title: c.title,
                spec: c.spec,
                cron: c.cron,
                tz: c.tz,
                persona_id: c.persona_id,
                session_id,
                next_run,
            })
            .await?;
        self.frame(&task, "task.created", ("task", task.to_json()));
        Ok(task)
    }

    /// `PATCH /tasks/{id}`: field edits, pause and resume; `next_run` is recalculated.
    pub async fn edit(&self, id: i64, body: &Value) -> TaskResult<Task> {
        let e = parse_edit(body).map_err(TaskError::Invalid)?;
        let cur = self.inner.store.get(id).await?;
        let cron = match e.cron {
            Some(c) => c,
            None => cur.cron.clone(),
        };
        let tz = e.tz.unwrap_or_else(|| cur.tz.clone());
        let state = e.state.unwrap_or_else(|| cur.state.clone());
        let mut errs: Vec<FieldError> = Vec::new();
        match (cur.kind.as_str(), &cron) {
            ("sched", None) => errs.push(FieldError::new("cron", "sched needs a schedule")),
            ("deleg", Some(_)) => errs.push(FieldError::new("cron", "deleg has no schedule")),
            (_, Some(c)) => match cron_check(c, &tz) {
                Ok(()) => {}
                Err(crate::CronError::Zone) => {
                    errs.push(FieldError::new("tz", "zone from the IANA database"))
                }
                Err(crate::CronError::Pattern) => {
                    errs.push(FieldError::new("cron", "five-field cron"));
                }
            },
            _ => {}
        }
        if !errs.is_empty() {
            return Err(TaskError::Invalid(errs));
        }
        let now = self.inner.clock.now_unix();
        let upd = TaskUpdate {
            title: e.title.unwrap_or_else(|| cur.title.clone()),
            spec: e.spec.unwrap_or_else(|| cur.spec.clone()),
            next_run: compute_next(&cur.kind, cron.as_deref(), &tz, &state, now),
            cron,
            tz,
            state,
            persona_id: e.persona_id.unwrap_or(cur.persona_id),
        };
        let task = self.inner.store.update(id, upd).await?;
        self.frame(&task, "task.updated", ("task", task.to_json()));
        Ok(task)
    }

    pub async fn delete(&self, id: i64) -> TaskResult<()> {
        self.inner.store.delete(id).await.map(|_| ())
    }

    pub async fn list(&self, f: TaskFilter) -> TaskResult<Vec<Task>> {
        self.inner.store.list(f).await
    }

    pub async fn runs(&self, id: i64, before: Option<i64>, limit: i64) -> TaskResult<Vec<TaskRun>> {
        self.inner.store.runs(id, before, limit).await
    }

    /// Background scheduler tick: stale runs left over from a crash are closed at start.
    pub fn spawn(&self, every: Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            let now = me.inner.clock.now_unix();
            if let Err(e) = me.inner.store.fail_stale_runs(now).await {
                tracing::warn!(scope = "tasks", event = "stale_runs_failed", error = ?e, "interrupted runs not closed");
            }
            loop {
                me.tick().await;
                tokio::time::sleep(every).await;
            }
        });
    }
}
