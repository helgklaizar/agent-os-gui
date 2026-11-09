//! `/secrets`: write-only (docs/reference/api.md 5.7, ADR-0003). The value is neither returned nor logged; the routes are
//! local-only for `admin` (table `table.rs`).

use std::sync::Arc;

use agentd_core::{
    FieldError, Secret, SecretError, SecretInfo, SecretStore, TreeError, account_of_ref,
    valid_secret_ref,
};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::Value;

use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, map_err, no_content, ok};

type St = State<Arc<ApiState>>;

fn store(state: &ApiState) -> Result<Arc<dyn SecretStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.secrets.clone())
        .ok_or_else(|| ApiError::core_down("Secret store unavailable"))
}

fn map_secret(e: SecretError) -> ApiError {
    match e {
        SecretError::Invalid(field) => invalid(vec![FieldError::new(field, "value not accepted")]),
        SecretError::Unavailable => ApiError::core_down("Secret store unavailable"),
    }
}

fn check_ref(r: &str) -> Result<(), ApiError> {
    if valid_secret_ref(r) {
        Ok(())
    } else {
        Err(invalid(vec![FieldError::new(
            "ref",
            "provider.<name> or account.<id> expected",
        )]))
    }
}

/// Body `{ "value": "<string>" }` and nothing else.
fn parse_value(body: &Value) -> Result<Secret<String>, ApiError> {
    let Some(obj) = body.as_object() else {
        return Err(invalid(vec![FieldError::new("", "object expected")]));
    };
    let mut errs: Vec<FieldError> = obj
        .keys()
        .filter(|k| *k != "value")
        .map(|k| FieldError::new(k, "unknown field"))
        .collect();
    match obj.get("value").and_then(Value::as_str) {
        Some(v) if !v.is_empty() => {
            if errs.is_empty() {
                return Ok(Secret::new(v.to_string()));
            }
        }
        _ => errs.push(FieldError::new("value", "a non-empty string is required")),
    }
    Err(invalid(errs))
}

async fn list(State(state): St) -> Result<Response, ApiError> {
    let items: Vec<SecretInfo> = store(&state)?.list().await.map_err(map_secret)?;
    Ok(ok(
        serde_json::to_value(items).map_err(|_| ApiError::internal())?
    ))
}

async fn put(State(state): St, Path(r): Path<String>, body: Body) -> Result<Response, ApiError> {
    let secrets = store(&state)?;
    check_ref(&r)?;
    let value = parse_value(&read_json(body).await?)?;
    // An account secret is bound to the account before the value is written: a nonexistent account gets no secret.
    // TODO(A4-03): `202` if `config_write` is in `ask` mode.
    if let Some(id) = account_of_ref(&r) {
        let tree = state
            .services()
            .and_then(|s| s.tree.clone())
            .ok_or_else(|| ApiError::core_down("Session store unavailable"))?;
        tree.link_account_secret(id, Some(r.clone()))
            .await
            .map_err(map_err)?;
    }
    let now = state.clock.now_unix();
    secrets.put(&r, value, now).await.map_err(map_secret)?;
    // Key written: verify it with a light provider request and update the account status (A3-09).
    if let (Some(id), Some(svc)) = (
        account_of_ref(&r),
        state.services().and_then(|s| s.responder.clone()),
    ) {
        svc.spawn_check(id);
    }
    // Channel token: the connection is verified by the channel provider (A5-06).
    if let (Some(id), Some(ch)) = (
        account_of_ref(&r),
        state.services().and_then(|s| s.channels.clone()),
    ) {
        tokio::spawn(async move { ch.refresh(id).await });
    }
    // The response per the contract is `SecretInfo`; the value is not returned.
    let info = SecretInfo {
        secret_ref: r,
        set: true,
        updated_at: now,
    };
    Ok(ok(
        serde_json::to_value(info).map_err(|_| ApiError::internal())?
    ))
}

async fn delete(State(state): St, Path(r): Path<String>) -> Result<Response, ApiError> {
    let secrets = store(&state)?;
    check_ref(&r)?;
    if !secrets.delete(&r).await.map_err(map_secret)? {
        return Err(map_err(TreeError::NotFound));
    }
    if let Some(id) = account_of_ref(&r)
        && let Some(tree) = state.services().and_then(|s| s.tree.clone())
    {
        // The account may have disappeared before the secret: then there is nothing to bind.
        match tree.link_account_secret(id, None).await {
            Ok(()) | Err(TreeError::NotFound) => {}
            Err(e) => return Err(map_err(e)),
        }
    }
    Ok(no_content())
}

pub(crate) fn routes() -> (
    axum::routing::MethodRouter<Arc<ApiState>>,
    axum::routing::MethodRouter<Arc<ApiState>>,
) {
    (
        axum::routing::get(list),
        axum::routing::put(put).delete(delete),
    )
}
