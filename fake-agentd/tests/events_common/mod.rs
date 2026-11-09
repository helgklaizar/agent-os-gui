//! Shared by the event tests: a mock without event fixtures, adding events, reading the feed and `events` frames.
#![allow(dead_code)]

pub use crate::chat_common::*;

use fake_agentd::{Config, events_db::NewEvent};
use serde_json::{Value, json};

/// Mock on a manual clock and without event fixtures: the feed is empty until a test fills it.
pub async fn bare() -> Rig {
    rig_with(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .no_event_fixtures(),
    )
    .await
}

impl Rig {
    /// Adds an event through the mock (insert and an `events` frame); returns its id.
    pub fn add(&self, kind: &str, sub: &str, text: &str, session: Option<i64>) -> i64 {
        let e = NewEvent {
            kind,
            sub,
            text,
            session_id: session,
            ts: self.d.now_unix(),
            ..NewEvent::default()
        };
        match self.d.add_event(&e) {
            Ok(Some(id)) => id,
            other => panic!("event not added: {other:?}"),
        }
    }

    pub async fn events(&self, query: &str) -> Value {
        let path = if query.is_empty() {
            "/v1/events".to_string()
        } else {
            format!("/v1/events?{query}")
        };
        self.get(&path).await
    }

    pub async fn ids(&self, query: &str) -> Vec<i64> {
        self.events(query).await["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e["id"].as_i64())
            .collect()
    }

    pub async fn read_all(&self, body: Value) -> Value {
        let r = self.call("POST", "/v1/events/read", Some(body)).await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        r.json()
    }
}

pub fn unread_total(v: &Value) -> i64 {
    v["counts"]["total"].as_i64().unwrap_or(-1)
}

pub fn empty_body() -> Value {
    json!({})
}
