//! `GET /permissions`, `PUT /permissions/{scope}/{scope_id}/{action}` (docs/reference/api.md 5.4, ADR-0005) and the session permission check
//! in `PATCH /sessions/{id}` (`cfg.perms`).

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{
    Action, FieldError, Level, Mode, PermEngine, Principal, PutError, Rule, TokenKind,
};
use axum::body::Body;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get, put};
use serde_json::{Value, json};

use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, ok};

type St = State<Arc<ApiState>>;

pub(crate) fn engine(state: &ApiState) -> Result<PermEngine, ApiError> {
    state
        .services()
        .and_then(|s| s.engine.clone())
        .ok_or_else(|| ApiError::core_down("Permission engine unavailable"))
}

/// Decisions and rules are accepted only from the local GUI: an `admin` token of kind `gui` (or `test` in the test build).
/// Routes already require `admin` and the local socket (table); the token kind is checked here a second time so that no
/// other token kind gets permissions even if it is ever issued `admin`.
pub(crate) fn require_gui(who: &Principal) -> Result<(), ApiError> {
    if matches!(who.kind, TokenKind::Gui | TokenKind::Test) {
        Ok(())
    } else {
        Err(ApiError::permission_denied("not_gui_token"))
    }
}

/// Initiator label for the log and findings: token kind and number, not its value.
pub(crate) fn who_label(who: &Principal) -> String {
    format!("{}:{}", who.kind.as_str(), who.token_id)
}

fn query(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

async fn list(State(state): St, RawQuery(raw): RawQuery) -> Result<Response, ApiError> {
    let engine = engine(&state)?;
    let q = query(raw);
    let level = match q.get("scope") {
        None => None,
        Some(v) => Some(
            Level::parse(v)
                .ok_or_else(|| ApiError::bad_request("scope: global, persona or session"))?,
        ),
    };
    let scope_id = match q.get("scope_id") {
        None => None,
        Some(v) => Some(
            v.parse::<i64>()
                .ok()
                .filter(|n| *n >= 0)
                .ok_or_else(|| ApiError::bad_request("scope_id: non-negative integer"))?,
        ),
    };
    let rules = engine
        .store()
        .list_rules(level, scope_id)
        .await
        .map_err(|_| ApiError::core_down("Permission store unavailable"))?;
    Ok(ok(Value::Array(rules.iter().map(Rule::to_json).collect())))
}

/// Body `{ "mode": "allow|ask|deny" }` and nothing else.
fn parse_mode(body: &Value) -> Result<Mode, ApiError> {
    let Some(obj) = body.as_object() else {
        return Err(invalid(vec![FieldError::new("", "object expected")]));
    };
    let mut errs: Vec<FieldError> = obj
        .keys()
        .filter(|k| *k != "mode")
        .map(|k| FieldError::new(k, "unknown field"))
        .collect();
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .and_then(Mode::parse);
    if mode.is_none() {
        errs.push(FieldError::new("mode", "allow, ask or deny expected"));
    }
    match (mode, errs.is_empty()) {
        (Some(m), true) => Ok(m),
        _ => Err(invalid(errs)),
    }
}

/// Rule from the path: level, `scope_id` (`0` for `global`), action.
fn rule_from_path(scope: &str, scope_id: &str, action: &str, mode: Mode) -> Result<Rule, ApiError> {
    let id: i64 = scope_id
        .parse()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| ApiError::bad_request("scope_id must be a non-negative integer"))?;
    let mut errs = Vec::new();
    let level = Level::parse(scope);
    if level.is_none() {
        errs.push(FieldError::new("scope", "global, persona or session"));
    }
    let action = Action::parse(action);
    if action.is_none() {
        errs.push(FieldError::new("action", "unknown action"));
    }
    if let Some(l) = level
        && (l == Level::Global) != (id == 0)
    {
        errs.push(FieldError::new("scope_id", "0 only for global"));
    }
    match (level, action, errs.is_empty()) {
        (Some(level), Some(action), true) => Ok(Rule {
            level,
            scope_id: id,
            action,
            mode,
        }),
        _ => Err(invalid(errs)),
    }
}

fn put_error(e: &PutError) -> ApiError {
    match e {
        PutError::NotFound => ApiError::not_found(),
        PutError::Loosen => ApiError::permission_denied("loosen"),
        PutError::Unavailable => ApiError::core_down("Permission store unavailable"),
    }
}

/// Increments the service counter of the `permissions` page and publishes `{page, rev}` to `settings`: clients re-read
/// the matrix. A failure does not break the rule already written, but shows up in the log.
pub(crate) async fn publish_rules_changed(state: &ApiState) {
    let Some(settings) = state.services().and_then(|s| s.settings.clone()) else {
        return;
    };
    for _ in 0..3 {
        let Ok(cur) = settings.get("permissions").await else {
            break;
        };
        let next = cur.doc["rules_rev"].as_u64().unwrap_or(0) + 1;
        match settings
            .patch("permissions", json!({ "rules_rev": next }), cur.rev)
            .await
        {
            Ok(saved) => {
                state
                    .hub
                    .publish("settings", json!({"page": "permissions", "rev": saved.rev}));
                return;
            }
            Err(agentd_core::SettingsError::Stale { .. }) => {}
            Err(_) => break,
        }
    }
    tracing::warn!(
        scope = "perms",
        event = "settings_delta_lost",
        "permissions page delta not published"
    );
}

async fn put_rule(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path((scope, scope_id, action)): Path<(String, String, String)>,
    body: Body,
) -> Result<Response, ApiError> {
    require_gui(&who)?;
    let engine = engine(&state)?;
    let mode = parse_mode(&read_json(body).await?)?;
    let rule = rule_from_path(&scope, &scope_id, &action, mode)?;
    let target = format!("{}/{}/{}", rule.level.as_str(), rule.scope_id, rule.action);
    let res = engine.put(rule, &who_label(&who)).await;
    let (decision, result) = match &res {
        Ok(_) => (None, "ok"),
        Err(PutError::Loosen) => (Some(agentd_core::Decision::Deny), "loosen_blocked"),
        Err(_) => (None, "failed"),
    };
    crate::audit::user_action_with(
        &state,
        &who,
        "permission_put",
        target,
        json!({"mode": mode.as_str()}),
        decision,
        result,
    )
    .await;
    let saved = res.map_err(|e| put_error(&e))?;
    publish_rules_changed(&state).await;
    Ok(ok(saved.to_json()))
}

/// `cfg.perms` in `PATCH /sessions/{id}`: each pair is written as a session rule; loosening gives `403` and a finding.
pub(crate) async fn apply_session_perms(
    state: &ApiState,
    who: &Principal,
    session_id: i64,
    perms: &serde_json::Map<String, Value>,
) -> Result<(), ApiError> {
    if perms.is_empty() {
        return Ok(());
    }
    let engine = engine(state)?;
    for (a, m) in perms {
        let (Some(action), Some(mode)) = (Action::parse(a), m.as_str().and_then(Mode::parse))
        else {
            return Err(invalid(vec![FieldError::new(
                "cfg.perms",
                "an object of the form action -> allow|ask|deny is required",
            )]));
        };
        let rule = Rule {
            level: Level::Session,
            scope_id: session_id,
            action,
            mode,
        };
        let res = engine.put(rule, &who_label(who)).await;
        let (decision, result) = match &res {
            Ok(_) => (None, "ok"),
            Err(PutError::Loosen) => (Some(agentd_core::Decision::Deny), "loosen_blocked"),
            Err(_) => (None, "failed"),
        };
        crate::audit::user_action_with(
            state,
            who,
            "permission_put",
            format!("session/{session_id}/{action}"),
            json!({"mode": mode.as_str(), "via": "cfg.perms"}),
            decision,
            result,
        )
        .await;
        res.map_err(|e| put_error(&e))?;
    }
    publish_rules_changed(state).await;
    Ok(())
}

pub(crate) fn list_route() -> MethodRouter<Arc<ApiState>> {
    get(list)
}

pub(crate) fn put_route() -> MethodRouter<Arc<ApiState>> {
    put(put_rule)
}
