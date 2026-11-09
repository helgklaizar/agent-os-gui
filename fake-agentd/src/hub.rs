//! WS frame broadcast: last state of topics, control commands to connections.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::FrameType;

#[derive(Debug, Clone)]
pub enum Out {
    Frame {
        topic: String,
        kind: FrameType,
        data: Value,
    },
    /// Drop the connection without a close frame.
    Drop,
    /// Close connections of this token with code 4401.
    Revoke(i64),
    /// The next topic frame gets `seq` one greater than expected.
    SkipSeq(String),
}

pub struct Hub {
    tx: broadcast::Sender<Out>,
    latest: Mutex<HashMap<String, Value>>,
    no_data: Mutex<HashSet<String>>,
    dropped: AtomicBool,
    pongs: AtomicBool,
    conns: AtomicUsize,
}

/// Topics from api.md section 4.3 (and `session.<id>`).
pub fn known_topic(topic: &str) -> bool {
    const FIXED: [&str; 9] = [
        "bar.sys",
        "bar.quota",
        "bar.weather",
        "bar.indicators",
        "events",
        "sessions",
        "modules",
        "settings",
        "connections",
    ];
    FIXED.contains(&topic)
        || topic
            .strip_prefix("session.")
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

fn default_state(topic: &str) -> Value {
    match topic {
        "bar.sys" => json!({ "cpu": 10, "gpu": 10, "ram": 50, "disk": 40, "temp": 40 }),
        "events" | "bar.indicators" => json!({ "items": [] }),
        _ => json!({}),
    }
}

impl Hub {
    pub fn new() -> Hub {
        let (tx, _) = broadcast::channel(256);
        Hub {
            tx,
            latest: Mutex::new(HashMap::new()),
            no_data: Mutex::new(HashSet::new()),
            dropped: AtomicBool::new(false),
            pongs: AtomicBool::new(true),
            conns: AtomicUsize::new(0),
        }
    }

    pub fn listen(&self) -> broadcast::Receiver<Out> {
        self.tx.subscribe()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, HashMap<String, Value>> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publishes a frame and remembers the state: a snapshot replaces, a delta adds, an event leaves the state unchanged.
    /// An explicit publication clears the "no data" mark on the topic.
    pub fn emit(&self, topic: &str, kind: FrameType, data: Value) {
        self.no_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(topic);
        self.publish(topic, kind, data);
    }

    /// Broadcasts a frame without touching the stored topic state (set by [`set_state`](Self::set_state)).
    pub fn emit_keep(&self, topic: &str, kind: FrameType, data: Value) {
        self.no_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(topic);
        let _ = self.tx.send(Out::Frame {
            topic: topic.to_string(),
            kind,
            data,
        });
    }

    /// Replaces the stored topic state: a new subscriber gets it as the first frame.
    pub fn set_state(&self, topic: &str, state: Value) {
        self.state().insert(topic.to_string(), state);
    }

    pub fn is_no_data(&self, topic: &str) -> bool {
        self.no_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(topic)
    }

    fn publish(&self, topic: &str, kind: FrameType, data: Value) {
        {
            let mut st = self.state();
            let cur = st
                .entry(topic.to_string())
                .or_insert_with(|| default_state(topic));
            match (kind, cur.as_object_mut(), data.as_object()) {
                (FrameType::Snapshot, _, _) => *cur = data.clone(),
                // api.md: the `bar.indicators` snapshot is `{ items: [IndicatorState] }`, a delta is one state (by `id`).
                (FrameType::Delta, Some(have), Some(patch)) if topic == "bar.indicators" => {
                    let items = have
                        .entry("items")
                        .or_insert_with(|| Value::Array(Vec::new()));
                    if let Some(list) = items.as_array_mut() {
                        match list.iter_mut().find(|i| i.get("id") == patch.get("id")) {
                            Some(Value::Object(slot)) => {
                                // a frame without value and text erases the previous ones: the value belongs to the last frame
                                slot.remove("value");
                                slot.remove("text");
                                slot.extend(patch.clone());
                            }
                            Some(slot) => *slot = data.clone(),
                            None => list.push(data.clone()),
                        }
                    }
                }
                (FrameType::Delta, Some(have), Some(patch)) => have.extend(patch.clone()),
                (FrameType::Delta, _, _) => *cur = data.clone(),
                (FrameType::Event, _, _) => {}
            }
        }
        let _ = self.tx.send(Out::Frame {
            topic: topic.to_string(),
            kind,
            data,
        });
    }

    pub fn snapshot(&self, topic: &str) -> Value {
        self.state()
            .get(topic)
            .cloned()
            .unwrap_or_else(|| default_state(topic))
    }

    /// "No data": all topic fields become `null`.
    pub fn no_data(&self, topic: &str) {
        let nulls = match self.snapshot(topic) {
            Value::Object(m) => Value::Object(m.keys().map(|k| (k.clone(), Value::Null)).collect()),
            _ => json!({}),
        };
        self.no_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(topic.to_string());
        self.publish(topic, FrameType::Delta, nulls);
    }

    pub fn control(&self, msg: Out) {
        let _ = self.tx.send(msg);
    }

    pub fn drop_ws(&self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.control(Out::Drop);
    }

    pub fn restore(&self) {
        self.dropped.store(false, Ordering::SeqCst);
    }

    pub fn is_dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }

    pub fn set_pongs(&self, on: bool) {
        self.pongs.store(on, Ordering::SeqCst);
    }

    pub fn pongs(&self) -> bool {
        self.pongs.load(Ordering::SeqCst)
    }

    pub fn conns(&self) -> usize {
        self.conns.load(Ordering::SeqCst)
    }

    pub fn conn_opened(&self) {
        self.conns.fetch_add(1, Ordering::SeqCst);
    }

    pub fn conn_closed(&self) {
        self.conns.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Default for Hub {
    fn default() -> Self {
        Hub::new()
    }
}
