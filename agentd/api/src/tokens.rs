//! `GET /v1/tokens`, `POST /v1/tokens`, `DELETE /v1/tokens/{id}` (A1-07): all routes are `admin` and local-only
//! (checked by the access layer via the table). The token value appears only in the issue response.

use std::sync::Arc;

use agentd_core::{
    ExternalTokenRequest, FieldError, Principal, RevokeOutcome, Scope, TokenAdmin, TokenAdminError,
};
use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;

fn admin(state: &ApiState) -> Result<Arc<dyn TokenAdmin>, ApiError> {
    state
        .services()
        .and_then(|s| s.tokens.clone())
        .ok_or_else(|| ApiError::core_down("Token store unavailable"))
}

fn map_err(e: TokenAdminError) -> ApiError {
    match e {
        TokenAdminError::Invalid(fields) => ApiError::validation_failed(&fields),
        TokenAdminError::Unavailable => ApiError::core_down("Token store unavailable"),
    }
}

pub(crate) async fn list(State(state): State<Arc<ApiState>>) -> Result<Response, ApiError> {
    let items = admin(&state)?.list().await.map_err(map_err)?;
    Ok(Json(items).into_response())
}

/// `TokenCreate` body: types are checked here, rules (scope not above `chat`, lifetime up to 90 days) are in the store.
fn parse_create(body: &Value, created_by: i64) -> Result<ExternalTokenRequest, ApiError> {
    let Some(obj) = body.as_object() else {
        return Err(ApiError::bad_request("A JSON object is expected"));
    };
    let mut errs: Vec<FieldError> = Vec::new();
    let name = match obj.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => {
            errs.push(FieldError::new("name", "a string is required"));
            String::new()
        }
    };
    let scope = match obj.get("scope").and_then(Value::as_str).map(Scope::parse) {
        Some(Some(s)) => s,
        _ => {
            errs.push(FieldError::new("scope", "read or chat expected"));
            Scope::Read
        }
    };
    let ttl_days = match obj.get("ttl_days") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64().and_then(|d| u32::try_from(d).ok()) {
            Some(d) => Some(d),
            None => {
                errs.push(FieldError::new(
                    "ttl_days",
                    "an integer from 1 to 90 is required",
                ));
                None
            }
        },
    };
    if obj
        .get("kind")
        .is_some_and(|k| k.as_str() != Some("external"))
    {
        errs.push(FieldError::new(
            "kind",
            "only external can be issued via the API",
        ));
    }
    if errs.is_empty() {
        Ok(ExternalTokenRequest {
            name,
            scope,
            ttl_days,
            created_by: Some(created_by),
        })
    } else {
        Err(ApiError::validation_failed(&errs))
    }
}

pub(crate) async fn create(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
    body: Body,
) -> Result<Response, ApiError> {
    let admin = admin(&state)?;
    let json = read_json(body).await?;
    let decide = json.get("decide").and_then(Value::as_bool) == Some(true);
    let req = parse_create(&json, who.token_id)?;
    let (name, scope, ttl) = (req.name.clone(), req.scope, req.ttl_days);
    let issued = if decide {
        admin.issue_web(req).await
    } else {
        admin.issue_external(req).await
    }
    .map_err(map_err)?;
    crate::audit::user_action(
        &state,
        &who,
        "token_issue",
        format!("token:{}", issued.info.id),
        serde_json::json!({"name": name, "scope": scope.as_str(), "kind": "external", "ttl_days": ttl, "decide": decide}),
    )
    .await;
    let mut doc = serde_json::to_value(&issued.info).map_err(|_| ApiError::internal())?;
    if let Value::Object(o) = &mut doc {
        o.insert("token".into(), Value::String(issued.value.expose().clone()));
    }
    Ok((StatusCode::CREATED, Json(doc)).into_response())
}

pub(crate) async fn revoke(
    State(state): State<Arc<ApiState>>,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id: i64 = id
        .parse()
        .map_err(|_| ApiError::bad_request("token id must be an integer"))?;
    match admin(&state)?.revoke(id).await.map_err(map_err)? {
        RevokeOutcome::NotFound => Err(ApiError::not_found()),
        RevokeOutcome::AlreadyRevoked => {
            // Closes connections left over from an earlier revocation if it went through another path.
            state.hub.revoke_token(id);
            Err(ApiError::conflict("already_revoked"))
        }
        RevokeOutcome::Revoked => {
            state.hub.revoke_token(id);
            crate::audit::user_action(
                &state,
                &who,
                "token_revoke",
                format!("token:{id}"),
                serde_json::json!({}),
            )
            .await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}
