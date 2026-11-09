//! `GET`/`PATCH /v1/settings/{page}` (including `theme` and `indicators`): `ETag`, `If-Match`, JSON merge patch.

use std::sync::Arc;

use agentd_core::{SettingsDoc, SettingsError, SettingsStore};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::body::{if_match_rev, read_json};
use crate::error::ApiError;
use crate::state::ApiState;

fn store(state: &ApiState) -> Result<Arc<dyn SettingsStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.settings.clone())
        .ok_or_else(|| ApiError::core_down("Settings store unavailable"))
}

fn map_err(e: SettingsError) -> ApiError {
    match e {
        SettingsError::UnknownPage => ApiError::not_found(),
        SettingsError::ReadOnly => ApiError::permission_denied("read_only"),
        SettingsError::Stale { current } => ApiError::precondition_failed(current),
        SettingsError::Invalid(fields) => ApiError::validation_failed(&fields),
        SettingsError::Unavailable => ApiError::core_down("Settings store unavailable"),
    }
}

/// Page document with `rev` in the body and `ETag` in the header.
fn page_response(d: SettingsDoc) -> Response {
    let etag = format!("\"{}\"", d.rev);
    let mut doc = d.doc;
    if let Value::Object(o) = &mut doc {
        o.insert("rev".into(), d.rev.into());
    }
    let mut resp = Json(doc).into_response();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    resp
}

pub(crate) async fn get_page(
    State(state): State<Arc<ApiState>>,
    Path(page): Path<String>,
) -> Result<Response, ApiError> {
    let mut doc = store(&state)?.get(&page).await.map_err(map_err)?;
    if page == "security" {
        // The state is computed: `ptrace_scope`, disk encryption, presence of secrets (without names and values).
        let secrets_present = match state.services().and_then(|s| s.secrets.clone()) {
            Some(s) => s.list().await.is_ok_and(|l| !l.is_empty()),
            None => false,
        };
        doc.doc = serde_json::json!({
            "ptrace_scope": agentd_sys::proc::ptrace_scope(),
            "disk_encryption": "unknown",
            "secrets_present": secrets_present,
        });
    }
    if page == "about" {
        // A read-only page: versions are computed, not stored (A6-12).
        let db_version = match state.services().and_then(|s| s.status.clone()) {
            Some(s) => s.user_version().await,
            None => None,
        };
        doc.doc = serde_json::json!({
            "core_version": env!("CARGO_PKG_VERSION"),
            "api": {"major": crate::health::API_MAJOR, "minor": crate::health::API_MINOR},
            "db_version": db_version,
            "capabilities": state.health.capabilities(),
        });
    }
    Ok(page_response(doc))
}

pub(crate) async fn patch_page(
    State(state): State<Arc<ApiState>>,
    Path(page): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let if_match = if_match_rev(&headers)?;
    let patch = read_json(body).await?;
    let saved = store.patch(&page, patch, if_match).await.map_err(map_err)?;
    // The `tunnels.api_external` setting turns TCP on and off without a restart (A1-04, A1-09).
    if page == "tunnels"
        && let Some(on) = saved.doc.get("api_external").and_then(Value::as_bool)
    {
        state.tcp.set_enabled(on);
    }
    // The account address or model changed: re-check the page's accounts (A3-09).
    if page == "models"
        && let Some(svc) = state.services().and_then(|s| s.responder.clone())
    {
        svc.spawn_check_models(&saved.doc);
    }
    state.hub.publish(
        "settings",
        serde_json::json!({ "page": page, "rev": saved.rev }),
    );
    Ok(page_response(saved))
}
