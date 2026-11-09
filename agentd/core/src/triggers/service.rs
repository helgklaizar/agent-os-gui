//! Trigger service: API edits and firing on incoming gateway messages with storm protection.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use super::{Trigger, TriggerError, TriggerResult, TriggerStore, parse_create, parse_edit};
use crate::{
    ChatService, Clock, EventLevel, Events, Gateway, IncomingHook, IncomingMsg, McpNotice,
    McpNoticeHook, NewEvent, PortFuture, RunAs, Tasks, UserMessage,
};

/// No more than this many firings of one trigger per window, then the trigger is paused.
pub const STORM_LIMIT: usize = 10;
pub const STORM_WINDOW_SECS: i64 = 60;

struct Inner {
    store: Arc<dyn TriggerStore>,
    clock: Arc<dyn Clock>,
    chat: ChatService,
    gateway: Gateway,
    events: Events,
    hits: Mutex<HashMap<i64, VecDeque<i64>>>,
    /// Tasks for the "create run" action; connected when the core is assembled.
    tasks: OnceLock<Tasks>,
}

/// Counter of MCP notifications: they have no message id (`gw_id = 0`), while a turn's `client_id` must be unique.
static MCP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct Triggers {
    inner: Arc<Inner>,
}

/// Whether a message matches the `match` conditions: `from` — sender or contact name, `contains` — substring
/// of the text; case-insensitive, an empty condition — any message.
fn matches(rule: &Value, msg: &IncomingMsg) -> bool {
    let norm = |s: &str| s.trim().to_lowercase();
    let from_ok = rule["from"].as_str().is_none_or(|f| {
        let f = norm(f);
        f == norm(&msg.handle) || msg.contact.as_deref().is_some_and(|c| f == norm(c))
    });
    let text_ok = rule["contains"]
        .as_str()
        .is_none_or(|c| norm(&msg.body).contains(&norm(c)));
    from_ok && text_ok
}

impl Triggers {
    #[must_use]
    pub fn new(
        store: Arc<dyn TriggerStore>,
        clock: Arc<dyn Clock>,
        chat: ChatService,
        gateway: Gateway,
        events: Events,
    ) -> Triggers {
        Triggers {
            inner: Arc::new(Inner {
                store,
                clock,
                chat,
                gateway,
                events,
                hits: Mutex::new(HashMap::new()),
                tasks: OnceLock::new(),
            }),
        }
    }

    /// Connects tasks: a trigger with `task_id` starts the task (once per core).
    pub fn set_tasks(&self, tasks: Tasks) {
        drop(self.inner.tasks.set(tasks));
    }

    /// A task for `task_id` exists; without connected tasks the action is unavailable.
    async fn check_task(&self, id: i64) -> TriggerResult<()> {
        let Some(tasks) = self.inner.tasks.get() else {
            return Err(TriggerError::TaskNotFound);
        };
        match tasks.store().get(id).await {
            Ok(_) => Ok(()),
            Err(_) => Err(TriggerError::TaskNotFound),
        }
    }

    pub async fn create(&self, body: &Value) -> TriggerResult<Trigger> {
        let c = parse_create(body).map_err(TriggerError::Invalid)?;
        if let Some(t) = c.task_id {
            self.check_task(t).await?;
        }
        self.inner
            .store
            .create(Trigger {
                id: 0,
                source: c.source,
                match_: c.match_,
                session_id: Some(c.session_id),
                task_id: c.task_id,
                enabled: c.enabled,
            })
            .await
    }

    pub async fn edit(&self, id: i64, body: &Value) -> TriggerResult<Trigger> {
        let e = parse_edit(body).map_err(TriggerError::Invalid)?;
        let cur = self.inner.store.get(id).await?;
        let task_id = e.task_id.unwrap_or(cur.task_id);
        if let Some(t) = task_id.filter(|t| Some(*t) != cur.task_id) {
            self.check_task(t).await?;
        }
        let enabled = e.enabled.unwrap_or(cur.enabled);
        if enabled && !cur.enabled {
            // Enabling after a pause starts the storm window anew.
            self.hits().remove(&id);
        }
        self.inner
            .store
            .update(
                id,
                Trigger {
                    id,
                    source: e.source.unwrap_or(cur.source),
                    match_: e.match_.unwrap_or(cur.match_),
                    session_id: e.session_id.or(cur.session_id),
                    task_id,
                    enabled,
                },
            )
            .await
    }

    pub async fn delete(&self, id: i64) -> TriggerResult<()> {
        self.hits().remove(&id);
        self.inner.store.delete(id).await
    }

    pub async fn list(&self) -> TriggerResult<Vec<Trigger>> {
        self.inner.store.list().await
    }

    fn hits(&self) -> std::sync::MutexGuard<'_, HashMap<i64, VecDeque<i64>>> {
        self.inner
            .hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Firing accounting: `false` if the window already has `STORM_LIMIT` of them.
    fn admit(&self, id: i64, now: i64) -> bool {
        let mut all = self.hits();
        let q = all.entry(id).or_default();
        while q.front().is_some_and(|t| now - *t >= STORM_WINDOW_SECS) {
            q.pop_front();
        }
        if q.len() >= STORM_LIMIT {
            return false;
        }
        q.push_back(now);
        true
    }

    async fn pause_for_storm(&self, t: &Trigger) {
        let mut paused = t.clone();
        paused.enabled = false;
        if self.inner.store.update(t.id, paused).await.is_err() {
            return;
        }
        let ev = NewEvent::new(
            "msg",
            "sys",
            EventLevel::Warn,
            format!(
                "Trigger {} paused: more than {STORM_LIMIT} firings per minute",
                t.id
            ),
        )
        .session(t.session_id);
        self.inner.events.emit(ev).await;
    }

    /// Trigger "create run" action: the task starts by the same path as `POST /tasks/{id}/run`.
    async fn run_task(&self, t: &Trigger, id: i64) {
        let started = match self.inner.tasks.get() {
            Some(tasks) => tasks.run_now(id).await.is_ok(),
            None => false,
        };
        if !started {
            let ev = NewEvent::new(
                "msg",
                "sys",
                EventLevel::Warn,
                format!("Trigger {}: task {id} not started", t.id),
            )
            .session(t.session_id);
            self.inner.events.emit(ev).await;
        }
    }

    /// `client_id` of a trigger turn: for channel messages — by message id (a redelivery spawns no extra turns), for
    /// MCP notifications — a separate label per notification.
    fn turn_client_id(&self, t: &Trigger, msg: &IncomingMsg) -> String {
        if msg.channel == MCP_SOURCE {
            let seq = MCP_SEQ.fetch_add(1, Ordering::Relaxed);
            format!("trigger-{}-mcp-{}-{seq}", t.id, self.inner.clock.now_unix())
        } else {
            format!("trigger-{}-{}", t.id, msg.gw_id)
        }
    }

    async fn fire(&self, t: &Trigger, msg: &IncomingMsg) {
        let Some(sid) = t.session_id else { return };
        if msg.channel == MCP_SOURCE {
            self.inner
                .gateway
                .deliver_notice(sid, &msg.handle, &msg.body)
                .await;
        } else {
            self.inner
                .gateway
                .deliver_to_session(sid, &msg.channel, msg.gw_id, &msg.body)
                .await;
        }
        if let Some(task) = t.task_id {
            self.run_task(t, task).await;
            return;
        }
        let note = UserMessage {
            client_id: self.turn_client_id(t, msg),
            body_md: format!(
                "Trigger fired: new incoming message ({}). Its text above is untrusted.",
                if msg.channel == MCP_SOURCE {
                    format!("MCP {}", msg.handle)
                } else {
                    msg.channel.clone()
                }
            ),
            attachment_ids: Vec::new(),
        };
        let origin = if msg.channel == MCP_SOURCE {
            format!("MCP server notification {}", msg.handle)
        } else {
            format!("message {}", msg.channel)
        };
        if self
            .inner
            .chat
            .send_as(sid, note, RunAs::trigger(&origin))
            .await
            .is_err()
        {
            tracing::warn!(
                scope = "triggers",
                event = "run_not_started",
                "trigger run not started"
            );
        }
    }
}

/// Trigger source for MCP server notifications: `match.from` — server name, `match.contains` — notification text.
pub const MCP_SOURCE: &str = "mcp";

impl McpNoticeHook for Triggers {
    fn on_notice<'a>(&'a self, n: &'a McpNotice) -> PortFuture<'a, ()> {
        Box::pin(async move {
            let msg = IncomingMsg {
                channel: MCP_SOURCE.to_string(),
                handle: n.server.clone(),
                contact: None,
                body: n.text.clone(),
                gw_id: 0,
            };
            self.dispatch(&msg).await;
        })
    }
}

impl IncomingHook for Triggers {
    fn on_incoming<'a>(&'a self, msg: &'a IncomingMsg) -> PortFuture<'a, ()> {
        Box::pin(self.dispatch(msg))
    }
}

impl Triggers {
    /// Finds enabled triggers of the source that match the conditions and starts them (with storm protection).
    async fn dispatch(&self, msg: &IncomingMsg) {
        let Ok(all) = self.inner.store.list().await else {
            return;
        };
        let now = self.inner.clock.now_unix();
        for t in all
            .iter()
            .filter(|t| t.enabled && t.source == msg.channel && matches(&t.match_, msg))
        {
            if self.admit(t.id, now) {
                self.fire(t, msg).await;
            } else {
                self.pause_for_storm(t).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(handle: &str, contact: Option<&str>, body: &str) -> IncomingMsg {
        IncomingMsg {
            channel: "tg".into(),
            handle: handle.into(),
            contact: contact.map(str::to_string),
            body: body.into(),
            gw_id: 1,
        }
    }

    #[test]
    fn from_matches_handle_or_contact_ignoring_case() {
        let m = msg(" @Vasya ", Some("Élodie"), "x");
        assert!(matches(&json!({"from": "@vasya"}), &m));
        assert!(matches(&json!({"from": "élodie"}), &m));
        assert!(!matches(&json!({"from": "@petya"}), &m));
    }

    #[test]
    fn contains_and_empty_rule() {
        let m = msg("@a", None, "URGENT: server");
        assert!(matches(&json!({"contains": "urgent"}), &m));
        assert!(!matches(&json!({"contains": "quiet"}), &m));
        assert!(
            matches(&json!({}), &m),
            "an empty condition matches any message"
        );
        assert!(
            !matches(&json!({"from": "@a", "contains": "quiet"}), &m),
            "both conditions at once"
        );
    }
}
