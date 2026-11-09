//! WS hub (docs/reference/api.md 4): topic registry, `delta`/`event` publishing, token revocation. One `broadcast` bus for all
//! connections: each connection selects its own topics and keeps its own `seq`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use agentd_core::{Clock, PortFuture, Scope};
use serde_json::Value;
use tokio::sync::broadcast;

/// Bus length: a lagging receiver loses messages and resynchronises with a fresh `snapshot`.
const BUS_LEN: usize = 1024;

/// Source of a topic's `snapshot`; `None` means the state is unavailable now (the client gets a `core_down` error).
pub trait TopicSource: Send + Sync + 'static {
    fn snapshot<'a>(&'a self, topic: &'a str) -> PortFuture<'a, Option<Value>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Delta,
    Event,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Delta => "delta",
            Kind::Event => "event",
        }
    }
}

#[derive(Clone)]
pub(crate) enum Bus {
    Message {
        topic: Arc<str>,
        kind: Kind,
        data: Arc<Value>,
        ts: i64,
    },
    Revoked(i64),
    /// User logout: close all connections with code `1001`.
    Logout,
}

#[derive(Clone)]
pub(crate) struct TopicEntry {
    pub min_scope: Scope,
    pub source: Arc<dyn TopicSource>,
}

struct Inner {
    clock: Arc<dyn Clock>,
    bus: broadcast::Sender<Bus>,
    exact: RwLock<HashMap<String, TopicEntry>>,
    /// A name ending in a dot (`session.`) defines a topic family (`session.42`).
    prefixes: RwLock<Vec<(String, TopicEntry)>>,
    /// Tokens of connections subscribed to the topic (one entry per connection).
    watchers: Mutex<HashMap<String, Vec<i64>>>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Hub {
        Hub {
            inner: Arc::new(Inner {
                clock,
                bus: broadcast::channel(BUS_LEN).0,
                exact: RwLock::new(HashMap::new()),
                prefixes: RwLock::new(Vec::new()),
                watchers: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Registers a topic: `min_scope` is what a token needs to subscribe (not below `read`), `source` provides the `snapshot`.
    /// A name with a trailing dot is a topic family.
    pub fn register_topic(&self, name: &str, min_scope: Scope, source: Arc<dyn TopicSource>) {
        let entry = TopicEntry { min_scope, source };
        if name.ends_with('.') {
            let mut p = self
                .inner
                .prefixes
                .write()
                .unwrap_or_else(|e| e.into_inner());
            p.retain(|(n, _)| n != name);
            p.push((name.to_string(), entry));
        } else {
            self.inner
                .exact
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(name.to_string(), entry);
        }
    }

    pub(crate) fn lookup(&self, topic: &str) -> Option<TopicEntry> {
        if let Some(e) = self
            .inner
            .exact
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(topic)
        {
            return Some(e.clone());
        }
        self.inner
            .prefixes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(p, _)| topic.len() > p.len() && topic.starts_with(p.as_str()))
            .map(|(_, e)| e.clone())
    }

    fn send(&self, topic: &str, kind: Kind, data: Value) {
        let msg = Bus::Message {
            topic: topic.into(),
            kind,
            data: Arc::new(data),
            ts: self.inner.clock.now_unix(),
        };
        // A send error means "no subscribers": there is nobody to publish to, it is not a failure.
        drop(self.inner.bus.send(msg));
    }

    /// `delta` to the topic's subscribers.
    pub fn publish(&self, topic: &str, data: Value) {
        self.send(topic, Kind::Delta, data);
    }

    /// `event` to the topic's subscribers (a one-off event, not a state change).
    pub fn publish_event(&self, topic: &str, data: Value) {
        self.send(topic, Kind::Event, data);
    }

    /// Closes all WS connections of this token with code `4401` (revocation).
    pub fn revoke_token(&self, token_id: i64) {
        drop(self.inner.bus.send(Bus::Revoked(token_id)));
    }

    /// Closes all WS connections with code `1001` (user logout).
    pub fn close_all(&self) {
        drop(self.inner.bus.send(Bus::Logout));
    }

    pub(crate) fn watch(&self, topic: &str, token_id: i64) {
        let mut w = self
            .inner
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        w.entry(topic.to_string()).or_default().push(token_id);
    }

    pub(crate) fn unwatch(&self, topic: &str, token_id: i64) {
        let mut w = self
            .inner
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(list) = w.get_mut(topic) {
            if let Some(pos) = list.iter().position(|t| *t == token_id) {
                list.swap_remove(pos);
            }
            if list.is_empty() {
                w.remove(topic);
            }
        }
    }

    /// How many connections are subscribed to the topic now.
    #[must_use]
    pub fn watchers(&self, topic: &str) -> usize {
        self.inner
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(topic)
            .map_or(0, Vec::len)
    }

    /// Tokens of connections subscribed to the topic, without repeats.
    #[must_use]
    pub fn watcher_tokens(&self, topic: &str) -> Vec<i64> {
        let mut v = self
            .inner
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(topic)
            .cloned()
            .unwrap_or_default();
        v.sort_unstable();
        v.dedup();
        v
    }

    pub(crate) fn bus(&self) -> broadcast::Receiver<Bus> {
        self.inner.bus.subscribe()
    }

    pub(crate) fn now(&self) -> i64 {
        self.inner.clock.now_unix()
    }
}

/// `snapshot` of the `settings` topic: `rev` of all written pages.
pub(crate) struct SettingsTopic(pub Arc<dyn agentd_core::SettingsStore>);

impl TopicSource for SettingsTopic {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            let pages = self.0.revisions().await.ok()?;
            Some(serde_json::json!({ "pages": pages }))
        })
    }
}

/// The hub as a store-event receiver: deltas go to topics after commit (`agentd_core::Notifier`).
impl agentd_core::Notifier for Hub {
    fn publish(&self, topic: &str, data: Value) {
        Hub::publish(self, topic, data);
    }

    fn publish_event(&self, topic: &str, data: Value) {
        Hub::publish_event(self, topic, data);
    }

    fn watchers(&self, topic: &str) -> usize {
        Hub::watchers(self, topic)
    }

    fn watcher_tokens(&self, topic: &str) -> Vec<i64> {
        Hub::watcher_tokens(self, topic)
    }
}
