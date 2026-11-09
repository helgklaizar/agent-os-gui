//! Event subtype registry: core data for `GET /events/subtypes`. Subtypes are not a DB schema enum (db.md section 3.3):
//! the GUI builds subfilters from this list, not from a hardcoded set.

use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtype {
    pub kind: String,
    pub sub: String,
    pub label: String,
    pub icon: Option<String>,
    pub enabled: bool,
}

impl Subtype {
    pub fn json(&self) -> Value {
        json!({
            "kind": self.kind, "sub": self.sub, "label": self.label,
            "icon": self.icon, "enabled": self.enabled,
        })
    }
}

/// Default registry: 11 subtypes (AI - 4, Tasks - 3, Messages - 4).
const DEFAULTS: [(&str, &str, &str, &str); 11] = [
    ("ai", "agent", "Agent", "system-run-symbolic"),
    ("ai", "watch", "Watcher", "view-reveal-symbolic"),
    ("ai", "claude", "Claude", "face-smile-symbolic"),
    ("ai", "local", "Local model", "computer-symbolic"),
    ("task", "sched", "Scheduled", "alarm-symbolic"),
    ("task", "deleg", "Delegated", "emblem-shared-symbolic"),
    ("task", "done", "Done", "emblem-ok-symbolic"),
    ("msg", "sys", "System", "dialog-information-symbolic"),
    ("msg", "wa", "WhatsApp", "chat-message-new-symbolic"),
    ("msg", "tg", "Telegram", "mail-send-symbolic"),
    ("msg", "mail", "Mail", "mail-unread-symbolic"),
];

pub struct Registry(Mutex<Vec<Subtype>>);

impl Registry {
    pub fn new() -> Registry {
        let items = DEFAULTS
            .iter()
            .map(|(kind, sub, label, icon)| Subtype {
                kind: (*kind).to_string(),
                sub: (*sub).to_string(),
                label: (*label).to_string(),
                icon: Some((*icon).to_string()),
                enabled: true,
            })
            .collect();
        Registry(Mutex::new(items))
    }

    /// A new entry or a replacement of the entry with the same `kind/sub` pair.
    pub fn register(&self, kind: &str, sub: &str, label: &str, icon: Option<&str>) {
        let item = Subtype {
            kind: kind.to_string(),
            sub: sub.to_string(),
            label: label.to_string(),
            icon: icon.map(str::to_string),
            enabled: true,
        };
        let mut items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match items.iter_mut().find(|s| s.kind == kind && s.sub == sub) {
            Some(old) => *old = item,
            None => items.push(item),
        }
    }

    pub fn list(&self) -> Vec<Subtype> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}
