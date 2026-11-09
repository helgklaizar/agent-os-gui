//! Core state for `GET /v1/health` and for `503 migrating` (docs/reference/api.md 2.1): status, version, compatibility, migration.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::layers::VERSION;

/// API version (`/v1`): `major` changes only on an incompatible change.
pub const API_MAJOR: u32 = 1;
pub const API_MINOR: u32 = 0;
/// Capabilities that are always present (A1-09: `if-match`, A1-10: `ws-seq`); the rest are toggled at
/// runtime (`HealthControl::set_capability`): `tool-results`, `terminal`, `observer-model` (A7-11).
pub const CAPABILITIES: &[&str] = &["if-match", "ws-seq"];
/// GUI version range: until the first GUI release, any `0.x`.
pub const MIN_CLIENT: &str = "0.0.0";
pub const MIN_GUI: &str = "0.0.0";
pub const MAX_GUI: &str = "0.x";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MigrationStatus {
    Running,
    Done,
    Failed,
}

/// DB migration progress (`migration` in `/health` and `details.migration` in `503 migrating`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MigrationView {
    pub status: MigrationStatus,
    pub from: i64,
    pub to: i64,
    pub step: Option<String>,
    pub progress: Option<f64>,
}

#[derive(Default)]
struct Inner {
    migration: Option<MigrationView>,
    ready: bool,
    degraded: bool,
    extra: std::collections::BTreeSet<&'static str>,
}

/// Shared part of the state: the core updates it, `/health` and the readiness layer read it.
#[derive(Clone, Default)]
pub struct HealthControl(Arc<Mutex<Inner>>);

impl HealthControl {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_migration(&self, migration: Option<MigrationView>) {
        self.lock().migration = migration;
    }

    /// Enables or disables a capability in the `capabilities` list.
    pub fn set_capability(&self, name: &'static str, on: bool) {
        let mut g = self.lock();
        if on {
            g.extra.insert(name);
        } else {
            g.extra.remove(name);
        }
    }

    /// Permanent and enabled capabilities, alphabetical without repeats.
    #[must_use]
    pub fn capabilities(&self) -> Vec<&'static str> {
        let g = self.lock();
        let mut all: Vec<&'static str> = CAPABILITIES.to_vec();
        all.extend(g.extra.iter().copied());
        all.sort_unstable();
        all.dedup();
        all
    }

    pub fn set_degraded(&self, degraded: bool) {
        self.lock().degraded = degraded;
    }

    pub(crate) fn set_ready(&self) {
        self.lock().ready = true;
    }

    pub(crate) fn status(&self) -> &'static str {
        let g = self.lock();
        match (&g.migration, g.ready) {
            (Some(m), false) if m.status != MigrationStatus::Done => "migrating",
            (_, false) => "starting",
            (_, true) if g.degraded => "degraded",
            (_, true) => "ok",
        }
    }

    pub(crate) fn migration_json(&self) -> Value {
        self.lock()
            .migration
            .as_ref()
            .and_then(|m| serde_json::to_value(m).ok())
            .unwrap_or(Value::Null)
    }

    pub(crate) fn body(&self, user_version: Option<i64>) -> Value {
        json!({
            "status": self.status(),
            "version": VERSION,
            "api": { "major": API_MAJOR, "minor": API_MINOR },
            "min_client": MIN_CLIENT,
            "min_gui": MIN_GUI,
            "max_gui": MAX_GUI,
            "user_version": user_version,
            "capabilities": self.capabilities(),
            "migration": self.migration_json(),
        })
    }
}

use std::sync::Arc as StdArc;

use axum::Json;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::state::ApiState;

/// `GET /v1/health`: anonymous, always `200`, fields per 2.1.
pub(crate) async fn handler(State(state): State<StdArc<ApiState>>) -> Json<Value> {
    let user_version = match state.services().and_then(|s| s.status.clone()) {
        Some(st) => st.user_version().await,
        None => None,
    };
    let mut body = state.health.body(user_version);
    // An unavailable observer degrades the core: dangerous actions are closed (fail-closed, ADR-0005).
    let observer_down = state
        .services()
        .and_then(|s| s.observer.clone())
        .is_some_and(|o| o.status() == agentd_core::ObserverStatus::Unavailable);
    if observer_down && body["status"] == "ok" {
        body["status"] = Value::from("degraded");
    }
    Json(body)
}

/// `GET /v1/metrics`: text exposition (`admin`, local; checked by the access layer).
pub(crate) async fn metrics(State(state): State<StdArc<ApiState>>) -> Response {
    let Some(status) = state.services().and_then(|s| s.status.clone()) else {
        return ApiError::core_down("DB state source not connected").into_response();
    };
    let text = state
        .metrics
        .render(status.write_queue_len(), status.user_version().await);
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], text).into_response()
}
