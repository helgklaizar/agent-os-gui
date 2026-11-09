//! `/modules` (docs/reference/api.md 5.6): catalog and installed modules, install (manifest, catalog entry; by URL after the address
//! check), edit, removal; the `modules` topic. The manifest is checked by the observer before install; removal and disabling
//! revoke the module's tokens. Only `admin` writes.

use std::collections::HashSet;
use std::sync::Arc;

use agentd_core::{
    FetchError, FieldError, Finding, Module, ModuleError, ModuleStore, PortFuture, Principal,
    check_install_url_allowing, validate_manifest,
};
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::{Value, json};

use crate::body::read_json;
use crate::error::ApiError;
use crate::hub::TopicSource;
use crate::state::ApiState;
use crate::tree::{created, invalid, no_content, ok};

type St = State<Arc<ApiState>>;
const MAX_CONFIG_BYTES: usize = 16 * 1024;

fn store(state: &ApiState) -> Result<Arc<dyn ModuleStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.modules.clone())
        .ok_or_else(|| ApiError::core_down("Modules unavailable"))
}

fn map_err(e: ModuleError) -> ApiError {
    match e {
        ModuleError::NotFound => ApiError::new(404, "not_found", "Module not found"),
        ModuleError::Exists => ApiError::conflict("already_installed"),
        ModuleError::Invalid(f) => invalid(f),
        ModuleError::Unavailable => ApiError::core_down("Modules unavailable"),
    }
}

/// Installed modules and catalog entries that are not among the installed ones.
async fn all(s: &dyn ModuleStore) -> Result<Vec<Module>, ApiError> {
    let mut items = s.list().await.map_err(map_err)?;
    let have: HashSet<String> = items.iter().map(|m| m.id.clone()).collect();
    for m in s.catalog().await {
        let module = Module::from_manifest(m, false);
        if !have.contains(&module.id) {
            items.push(module);
        }
    }
    Ok(items)
}

async fn list(State(state): St) -> Result<Response, ApiError> {
    let items = all(store(&state)?.as_ref()).await?;
    Ok(ok(Value::Array(
        items.iter().map(Module::to_json).collect(),
    )))
}

/// Manifest from the body: `manifest` (object), `catalog_id` or `url`; exactly one of the three.
/// `host:port` address allowed for download (test build only: file from `AGENTD_MODULE_ALLOW_FILE`).
fn allowed_host() -> Option<String> {
    agentd_core::manual_clock(0)?;
    let path = std::env::var_os("AGENTD_MODULE_ALLOW_FILE")?;
    let t = std::fs::read_to_string(path).ok()?;
    Some(t.trim().to_string()).filter(|s| !s.is_empty())
}

async fn manifest_of(
    state: &ApiState,
    s: &dyn ModuleStore,
    body: &Value,
) -> Result<Value, ApiError> {
    let keys: Vec<&str> = ["manifest", "catalog_id", "url"]
        .into_iter()
        .filter(|k| body.get(k).is_some())
        .collect();
    if keys.len() != 1 {
        return Err(invalid(vec![FieldError::new(
            "manifest",
            "one of manifest, catalog_id, url is required",
        )]));
    }
    match keys[0] {
        "manifest" => Ok(body["manifest"].clone()),
        "catalog_id" => {
            let id = body["catalog_id"].as_str().unwrap_or_default();
            s.catalog()
                .await
                .into_iter()
                .find(|m| m["id"] == id)
                .ok_or_else(|| ApiError::new(404, "not_found", "No such catalog entry"))
        }
        _ => {
            let url = body["url"].as_str().unwrap_or_default();
            let allowed = allowed_host();
            check_install_url_allowing(url, allowed.as_deref()).map_err(|e| invalid(vec![e]))?;
            let fetcher = state
                .services()
                .and_then(|s| s.module_fetcher.clone())
                .ok_or_else(|| ApiError::core_down("Module download unavailable"))?;
            let bytes = fetcher
                .fetch(url, allowed.as_deref())
                .await
                .map_err(|e| match e {
                    FetchError::Failed => {
                        ApiError::new(502, "bad_gateway", "Failed to download the manifest")
                    }
                    FetchError::Status(c) => invalid(vec![FieldError::new(
                        "url",
                        &format!("server responded {c}"),
                    )]),
                    FetchError::TooLarge => {
                        invalid(vec![FieldError::new("url", "manifest larger than 64 KiB")])
                    }
                })?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| invalid(vec![FieldError::new("url", "response is not JSON")]))
        }
    }
}

async fn publish(state: &ApiState, m: &Module) {
    state.hub.publish("modules", m.to_json());
}

async fn create(
    State(state): St,
    Extension(who): Extension<Principal>,
    body: Body,
) -> Result<Response, ApiError> {
    let s = store(&state)?;
    let manifest = manifest_of(&state, s.as_ref(), &read_json(body).await?).await?;
    let errs = validate_manifest(&manifest);
    if !errs.is_empty() {
        return Err(invalid(errs));
    }
    let module = Module::from_manifest(manifest, true);
    // Observer check before install: injection markers in the title, version and questions (manifest texts).
    let svc = state.services();
    if let Some(obs) = svc.as_ref().and_then(|s| s.observer.clone())
        && let Some(hit) = obs.check_text(&module.manifest.to_string())
    {
        if let Some(audit) = svc.as_ref().and_then(|s| s.audit.clone()) {
            let f = Finding {
                kind: "module",
                severity: hit.severity,
                ref_table: Some("modules"),
                ref_id: Some(module.id.clone()),
                verdict: "blocked",
                detail: Some(format!("rule={}", hit.rule)),
            };
            if audit.finding(f).await.is_err() {
                tracing::warn!(
                    scope = "observer",
                    event = "finding_lost",
                    "finding not recorded"
                );
            }
        }
        return Err(ApiError::permission_denied("observer_blocked"));
    }
    let saved = s.insert(module).await.map_err(map_err)?;
    crate::audit::user_action(
        &state,
        &who,
        "config_write",
        format!("module:{}", saved.id),
        json!({"op": "install", "permissions": saved.permissions().len()}),
    )
    .await;
    publish(&state, &saved).await;
    Ok(created(saved.to_json()))
}

/// Revokes the module's tokens and closes their WS.
async fn revoke_tokens(state: &ApiState, id: &str) {
    if let Some(tokens) = state.services().and_then(|s| s.tokens.clone())
        && let Ok(ids) = tokens.revoke_module(id).await
    {
        for t in ids {
            state.hub.revoke_token(t);
        }
    }
}

async fn patch(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let s = store(&state)?;
    let patch = read_json(body).await?;
    let mut m = s.get(id.clone()).await.map_err(map_err)?;
    let mut errs = Vec::new();
    for k in patch.as_object().into_iter().flatten().map(|(k, _)| k) {
        if !["enabled", "placement", "config"].contains(&k.as_str()) {
            errs.push(FieldError::new(k, "unknown field"));
        }
    }
    if let Some(v) = patch.get("enabled") {
        match v.as_bool() {
            Some(b) => m.enabled = b,
            None => errs.push(FieldError::new("enabled", "true or false")),
        }
    }
    if let Some(v) = patch.get("placement") {
        match v.as_str().filter(|p| agentd_core::PLACEMENTS.contains(p)) {
            Some(p) => m.placement = p.to_string(),
            None => errs.push(FieldError::new("placement", "panel, widget or button")),
        }
    }
    if let Some(v) = patch.get("config") {
        if v.is_object() && v.to_string().len() <= MAX_CONFIG_BYTES {
            let mut found = Vec::new();
            agentd_core::secret_fields(v, "config", &mut found);
            errs.extend(found);
            m.config = v.clone();
        } else {
            errs.push(FieldError::new("config", "object up to 16 KiB"));
        }
    }
    if !errs.is_empty() {
        return Err(invalid(errs));
    }
    let was_enabled = s.get(id.clone()).await.map_err(map_err)?.enabled;
    let saved = s.update(m).await.map_err(map_err)?;
    if was_enabled && !saved.enabled {
        revoke_tokens(&state, &saved.id).await;
    }
    crate::audit::user_action(
        &state,
        &who,
        "config_write",
        format!("module:{}", saved.id),
        json!({"op": "update"}),
    )
    .await;
    publish(&state, &saved).await;
    Ok(ok(saved.to_json()))
}

async fn delete(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let s = store(&state)?;
    let cur = s.get(id.clone()).await.map_err(map_err)?;
    s.delete(id.clone()).await.map_err(map_err)?;
    revoke_tokens(&state, &id).await;
    crate::audit::user_action(
        &state,
        &who,
        "config_write",
        format!("module:{id}"),
        json!({"op": "remove"}),
    )
    .await;
    // A removed module returns to the catalog (if it is there): the client gets a "not installed" entry.
    let mut gone = Module::from_manifest(cur.manifest, false);
    gone.enabled = false;
    publish(&state, &gone).await;
    Ok(no_content())
}

/// Panel token lifetime, days (assumption A6-04).
const PANEL_TOKEN_DAYS: u32 = 1;

/// `POST /modules/{module_id}/token`: token of an open panel; only for an installed and enabled module.
async fn token(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let m = store(&state)?.get(id).await.map_err(map_err)?;
    if !m.enabled {
        return Err(ApiError::conflict("module_disabled"));
    }
    let tokens = state
        .services()
        .and_then(|s| s.tokens.clone())
        .ok_or_else(|| ApiError::core_down("Tokens unavailable"))?;
    let issued = tokens
        .issue_module(&m.id, m.permissions(), PANEL_TOKEN_DAYS)
        .await
        .map_err(|_| ApiError::core_down("Tokens unavailable"))?;
    crate::audit::user_action(
        &state,
        &who,
        "config_write",
        format!("module:{}", m.id),
        json!({"op": "token", "token_id": issued.id}),
    )
    .await;
    Ok(created(
        json!({"token": issued.value.expose(), "expires_at": issued.expires_at}),
    ))
}

pub(crate) fn token_route() -> MethodRouter<Arc<ApiState>> {
    axum::routing::post(token)
}

pub(crate) struct ModulesTopic(pub Arc<dyn ModuleStore>);

impl TopicSource for ModulesTopic {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            let items = all(self.0.as_ref()).await.ok()?;
            Some(json!({"items": items.iter().map(Module::to_json).collect::<Vec<_>>()}))
        })
    }
}

pub(crate) fn routes() -> (MethodRouter<Arc<ApiState>>, MethodRouter<Arc<ApiState>>) {
    (
        get(list).post(create),
        axum::routing::patch(patch).delete(delete),
    )
}
