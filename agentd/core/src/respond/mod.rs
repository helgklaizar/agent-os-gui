//! Reply orchestration: the session message queue, context, provider, `message.delta` stream, stop.
//! Domain core: ports (`ChatStore`, `TreeStore`, `SettingsStore`, `QuotaStore`, `Notifier`) and providers are swappable.

mod check;
mod dispatch;
mod events;
mod origin;
mod prompt;
mod sink;
mod stream;
mod turn;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::Notify;

use origin::Origin;
pub use origin::{RunAs, TurnEnd};

use crate::{
    ChatError, ChatResult, ChatStore, Executors, Notifier, PolicyGate, Posted, ProviderRegistry,
    QuotaStore, SettingsStore, TreeStore, UserMessage,
};

/// What is needed to run agent actions: the permission gate and executors.
#[derive(Clone)]
pub(crate) struct Actions {
    pub gate: PolicyGate,
    pub executors: Executors,
    /// Agent MCP tools; without them `mcp__*` names are rejected as unknown.
    pub mcp: Option<crate::Mcp>,
}

/// Defaults: flush to the DB at least every 250 ms or 4 KiB, a reply lasts at most 10 minutes, the queue holds up to 10 messages.
#[derive(Debug, Clone, Copy)]
pub struct ChatConfig {
    pub flush_every: Duration,
    pub flush_bytes: usize,
    pub total_timeout: Duration,
    pub queue_max: usize,
    pub max_tokens: u32,
}

impl Default for ChatConfig {
    fn default() -> ChatConfig {
        ChatConfig {
            flush_every: Duration::from_millis(250),
            flush_bytes: 4096,
            total_timeout: Duration::from_secs(600),
            queue_max: 10,
            max_tokens: 4096,
        }
    }
}

#[derive(Default)]
struct Run {
    queue: VecDeque<i64>,
    busy: bool,
    stop: Option<Arc<Notify>>,
    origins: HashMap<i64, Origin>,
}

pub(crate) struct Inner {
    pub store: Arc<dyn ChatStore>,
    pub tree: Arc<dyn TreeStore>,
    pub settings: Arc<dyn SettingsStore>,
    pub providers: ProviderRegistry,
    pub quotas: Option<Arc<dyn QuotaStore>>,
    pub notifier: Arc<dyn Notifier>,
    pub cfg: ChatConfig,
    /// Agent actions; without them a tool call is rejected (`actions_unavailable`, fail-closed).
    pub actions: Option<Actions>,
    /// Event feed: a reply in an inactive session and provider failures.
    pub events: Option<crate::Events>,
    runs: Mutex<HashMap<i64, Run>>,
}

/// Chat service: accepts user messages and replies in order, one reply per session.
#[derive(Clone)]
pub struct ChatService {
    inner: Arc<Inner>,
}

pub(crate) fn topic(session_id: i64) -> String {
    format!("session.{session_id}")
}

impl ChatService {
    #[must_use]
    pub fn new(
        store: Arc<dyn ChatStore>,
        tree: Arc<dyn TreeStore>,
        settings: Arc<dyn SettingsStore>,
        providers: ProviderRegistry,
        quotas: Option<Arc<dyn QuotaStore>>,
        notifier: Arc<dyn Notifier>,
        cfg: ChatConfig,
    ) -> ChatService {
        ChatService {
            inner: Arc::new(Inner {
                store,
                tree,
                settings,
                providers,
                quotas,
                notifier,
                cfg,
                actions: None,
                events: None,
                runs: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Attaches agent actions: tool calls go through the permission gate and executors. Called right after
    /// `new`, before the service is handed out.
    #[must_use]
    pub fn with_actions(mut self, gate: PolicyGate, executors: Executors) -> ChatService {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.actions = Some(Actions {
                gate,
                executors,
                mcp: None,
            });
        }
        self
    }

    /// Attaches the MCP client: `mcp__<server>__<tool>` tools go through the same permission gate. Called after
    /// `with_actions`, before the service is handed out.
    #[must_use]
    pub fn with_mcp(mut self, mcp: crate::Mcp) -> ChatService {
        if let Some(inner) = Arc::get_mut(&mut self.inner)
            && let Some(a) = inner.actions.as_mut()
        {
            a.mcp = Some(mcp);
        }
        self
    }

    /// Attaches the event feed. Called before the service is handed out, like `with_actions`.
    #[must_use]
    pub fn with_events(mut self, events: crate::Events) -> ChatService {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.events = Some(events);
        }
        self
    }

    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Run>> {
        self.inner.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Core start: `streaming` messages left after a crash become `interrupted`.
    pub async fn recover(&self) -> ChatResult<u64> {
        let n = self.inner.store.interrupt_streaming().await?;
        if n > 0 {
            tracing::warn!(
                scope = "chat",
                event = "interrupted_on_start",
                count = n,
                "replies interrupted by a core crash"
            );
        }
        Ok(n)
    }

    /// User message: stored idempotently; a new one is queued for the session reply.
    /// Queue full: `QueueFull` before the message is stored.
    pub async fn send(&self, session_id: i64, msg: UserMessage) -> ChatResult<Posted> {
        self.send_as(session_id, msg, RunAs::agent())
            .await
            .map(|(p, _)| p)
    }

    /// Same, but agent actions in this turn run on behalf of `actor` (a task run is `Cron`); along with the intake result
    /// the turn outcome receiver is returned (`None` if the message already existed: a repeated `client_id`).
    pub async fn send_as(
        &self,
        session_id: i64,
        msg: UserMessage,
        run_as: RunAs,
    ) -> ChatResult<(Posted, Option<tokio::sync::oneshot::Receiver<TurnEnd>>)> {
        {
            let runs = self.runs();
            if runs
                .get(&session_id)
                .is_some_and(|r| r.queue.len() >= self.inner.cfg.queue_max)
            {
                return Err(ChatError::QueueFull);
            }
        }
        let posted = self.inner.store.post_user(session_id, msg).await?;
        let mut waiter = None;
        if let Posted::Created(m) = &posted {
            let id = m["id"].as_i64().unwrap_or(0);
            self.inner.notifier.publish_event(
                &topic(session_id),
                json!({"kind": "message.created", "message": m}),
            );
            let start = {
                let mut runs = self.runs();
                let run = runs.entry(session_id).or_default();
                run.queue.push_back(id);
                let (tx, rx) = tokio::sync::oneshot::channel();
                run.origins.insert(
                    id,
                    Origin {
                        run_as,
                        done: Some(tx),
                    },
                );
                waiter = Some(rx);
                !std::mem::replace(&mut run.busy, true)
            };
            if start {
                let me = self.clone();
                tokio::spawn(async move { me.worker(session_id).await });
            }
        }
        Ok((posted, waiter))
    }

    /// Stops the session's current reply; `false` if there was nothing to stop (repeating is safe).
    #[must_use]
    pub fn stop(&self, session_id: i64) -> bool {
        let stop = self.runs().get(&session_id).and_then(|r| r.stop.clone());
        stop.map(|s| s.notify_one()).is_some()
    }

    async fn worker(&self, session_id: i64) {
        loop {
            let (next, stop, origin) = {
                let mut runs = self.runs();
                let popped = runs.entry(session_id).or_default().queue.pop_front();
                let Some(id) = popped else {
                    // Queue empty: the session is free. The entry is removed under the same lock as `send`.
                    runs.remove(&session_id);
                    return;
                };
                let stop = Arc::new(Notify::new());
                if let Some(run) = runs.get_mut(&session_id) {
                    run.stop = Some(stop.clone());
                }
                let origin = runs
                    .get_mut(&session_id)
                    .and_then(|r| r.origins.remove(&id));
                (id, stop, origin)
            };
            let run_as = origin
                .as_ref()
                .map_or_else(RunAs::agent, |o| o.run_as.clone());
            let end = turn::respond(&self.inner, session_id, next, stop, run_as).await;
            if let Some(tx) = origin.and_then(|o| o.done) {
                drop(tx.send(end));
            }
            if let Some(r) = self.runs().get_mut(&session_id) {
                r.stop = None;
            }
        }
    }
}
