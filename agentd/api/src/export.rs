//! `POST /sessions/{id}/export` (docs/reference/api.md 5.1, docs/reference/db.md 7): encrypted session export; `age` recipients are the page
//! `schedule`, `backup.recipient.primary` and `backup.recipient.recovery`. No recipients gives `422 backup_not_configured`.

use std::sync::Arc;

use agentd_core::{ExportError, Principal};
use axum::extract::{Extension, Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, post};
use serde_json::json;

use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{ok, parse_id};

fn reasoned(reason: &str) -> ApiError {
    ApiError::new(422, "validation_failed", "Export is not configured")
        .details(json!({"reason": reason}))
}

async fn export(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_id(&id)?;
    let services = state
        .services()
        .ok_or_else(|| ApiError::core_down("Export unavailable"))?;
    let (Some(exporter), Some(settings)) = (services.exporter.clone(), services.settings.clone())
    else {
        return Err(ApiError::core_down("Export unavailable"));
    };
    let page = settings
        .get("schedule")
        .await
        .map_err(|_| ApiError::core_down("Settings unavailable"))?;
    let rec = &page.doc["backup"]["recipient"];
    let recipients: Vec<String> = ["primary", "recovery"]
        .iter()
        .filter_map(|k| rec[k].as_str().map(str::trim).filter(|s| !s.is_empty()))
        .map(str::to_string)
        .collect();
    if recipients.is_empty() {
        return Err(reasoned("backup_not_configured"));
    }
    let info = exporter
        .export(id, recipients, state.clock.now_unix())
        .await
        .map_err(|e| match e {
            ExportError::NotFound => ApiError::new(404, "not_found", "Session not found"),
            ExportError::BadRecipient => reasoned("backup_recipient_invalid"),
            ExportError::Unavailable => ApiError::core_down("Export unavailable"),
        })?;
    crate::audit::user_action(
        &state,
        &who,
        "read",
        format!("export:session:{id}"),
        json!({"size": info.size}),
    )
    .await;
    Ok(ok(json!({"path": info.path, "size": info.size})))
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    post(export)
}
