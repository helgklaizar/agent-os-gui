//! `GET /permission-requests/{id}` and `POST /permission-requests/{id}/decision` (docs/reference/api.md 5.4, ADR-0005).

use std::sync::Arc;

use agentd_core::{
    DecideError, DecideInput, DecisionKind, FieldError, Level, Mode, PermissionService, Principal,
    RuleInput,
};
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get, post};
use serde_json::{Value, json};

use crate::body::read_json;
use crate::error::ApiError;
use crate::permissions::{require_gui, who_label};
use crate::state::ApiState;
use crate::tree::{invalid, ok, parse_id};

type St = State<Arc<ApiState>>;

pub(crate) fn service(state: &ApiState) -> Result<PermissionService, ApiError> {
    state
        .services()
        .and_then(|s| s.gate.clone())
        .map(|g| g.requests().clone())
        .ok_or_else(|| ApiError::core_down("Permission requests unavailable"))
}

async fn get_one(State(state): St, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (svc, id) = (service(&state)?, parse_id(&id)?);
    match svc.get(id).await {
        Ok(Some(r)) => Ok(ok(r.to_json())),
        Ok(None) => Err(ApiError::not_found()),
        Err(_) => Err(ApiError::core_down("Permission store unavailable")),
    }
}

fn err(field: &str, msg: &str) -> FieldError {
    FieldError::new(field, msg)
}

/// "Always" rule: `scope` is persona or global (`session` is not allowed), `scope_id` (`0` for `global`), `mode`
/// defaults to `allow`.
fn parse_rule(v: &Value, errs: &mut Vec<FieldError>) -> Option<RuleInput> {
    let Some(o) = v.as_object() else {
        errs.push(err("rule", "object expected"));
        return None;
    };
    for k in o
        .keys()
        .filter(|k| !matches!(k.as_str(), "scope" | "scope_id" | "mode"))
    {
        errs.push(err(&format!("rule.{k}"), "unknown field"));
    }
    let level = match o
        .get("scope")
        .and_then(Value::as_str)
        .and_then(Level::parse)
    {
        Some(l @ (Level::Persona | Level::Global)) => Some(l),
        _ => {
            errs.push(err("rule.scope", "persona or global expected"));
            None
        }
    };
    let id = o.get("scope_id").and_then(Value::as_i64);
    let scope_id = match (level, id) {
        (Some(Level::Global), None | Some(0)) => Some(0),
        (Some(Level::Persona), Some(n)) if n > 0 => Some(n),
        (Some(_), _) => {
            errs.push(err(
                "rule.scope_id",
                "0 for global, persona number for persona",
            ));
            None
        }
        (None, _) => None,
    };
    let mode = match o.get("mode") {
        None | Some(Value::Null) => Some(Mode::Allow),
        Some(m) => {
            let parsed = m.as_str().and_then(Mode::parse);
            if parsed.is_none() {
                errs.push(err("rule.mode", "allow, ask or deny expected"));
            }
            parsed
        }
    };
    match (level, scope_id, mode) {
        (Some(level), Some(scope_id), Some(mode)) => Some(RuleInput {
            level,
            scope_id,
            mode,
        }),
        _ => None,
    }
}

/// Body `{ decision: allow|deny|always, rule? }`; `rule` is required for `always` and not allowed otherwise.
fn parse_decision(body: &Value) -> Result<DecisionKind, ApiError> {
    let Some(o) = body.as_object() else {
        return Err(invalid(vec![err("", "object expected")]));
    };
    let mut errs: Vec<FieldError> = o
        .keys()
        .filter(|k| !matches!(k.as_str(), "decision" | "rule"))
        .map(|k| err(k, "unknown field"))
        .collect();
    let decision = o.get("decision").and_then(Value::as_str);
    let rule_v = o.get("rule").filter(|v| !v.is_null());
    let kind = match (decision, rule_v) {
        (Some("allow"), None) => Some(DecisionKind::Allow),
        (Some("deny"), None) => Some(DecisionKind::Deny),
        (Some("allow" | "deny"), Some(_)) => {
            errs.push(err("rule", "rule only with decision = always"));
            None
        }
        (Some("always"), None) => {
            errs.push(err("rule", "always requires a rule"));
            None
        }
        (Some("always"), Some(v)) => parse_rule(v, &mut errs).map(DecisionKind::Always),
        _ => {
            errs.push(err("decision", "allow, deny or always expected"));
            None
        }
    };
    match (kind, errs.is_empty()) {
        (Some(k), true) => Ok(k),
        _ => Err(invalid(errs)),
    }
}

fn map_decide(e: DecideError) -> ApiError {
    match e {
        DecideError::NotFound => ApiError::not_found(),
        DecideError::AlreadyDecided(st) => ApiError::conflict("already_decided")
            .details(json!({"reason": "already_decided", "status": st.as_str()})),
        DecideError::Loosen => ApiError::permission_denied("loosen"),
        DecideError::RuleScopeNotFound => invalid(vec![err("rule.scope_id", "no such persona")]),
        DecideError::Unavailable => ApiError::core_down("Permission store unavailable"),
    }
}

/// Who decides: `Ok(false)` is the GUI (`gui`/`test`), `Ok(true)` is a web token with the `@decide` grant (ADR-0010 item 4), otherwise denied.
fn decider(who: &Principal) -> Result<bool, ApiError> {
    match require_gui(who) {
        Ok(()) => Ok(false),
        Err(e) if who.can_decide() => {
            drop(e);
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

/// A web token decides once (allow/deny); the "Always" rule changes permissions and stays with the GUI.
fn forbid_web_always(web: bool, always: bool) -> Result<(), ApiError> {
    if web && always {
        Err(ApiError::permission_denied("always_needs_gui"))
    } else {
        Ok(())
    }
}

/// Only a local `admin` of kind `gui` decides; other tokens and TCP are cut off by the route table.
async fn decide(
    State(state): St,
    Extension(who): Extension<Principal>,
    Path(id): Path<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let web = decider(&who)?;
    let (svc, id) = (service(&state)?, parse_id(&id)?);
    let kind = parse_decision(&read_json(body).await?)?;
    forbid_web_always(web, matches!(kind, DecisionKind::Always(_)))?;
    let input = DecideInput {
        kind,
        decided_by: who_label(&who),
    };
    let done = svc.decide(id, input).await.map_err(map_decide)?;
    if matches!(kind, DecisionKind::Always(_)) {
        crate::permissions::publish_rules_changed(&state).await;
    }
    Ok(ok(done.to_json()))
}

pub(crate) fn decision_route() -> MethodRouter<Arc<ApiState>> {
    post(decide)
}

pub(crate) fn get_route() -> MethodRouter<Arc<ApiState>> {
    get(get_one)
}

#[cfg(test)]
mod decider_tests {
    use super::*;
    use agentd_core::{Scope, TokenKind};

    fn who(kind: TokenKind, scope: Scope) -> Principal {
        Principal {
            token_id: 1,
            kind,
            scope,
            allowlist: None,
            expires_at: None,
        }
    }

    #[test]
    fn gui_and_test_decide_everything() {
        assert!(matches!(
            decider(&who(TokenKind::Gui, Scope::Admin)),
            Ok(false)
        ));
        assert!(matches!(
            decider(&who(TokenKind::Test, Scope::Admin)),
            Ok(false)
        ));
        assert!(forbid_web_always(false, true).is_ok());
    }

    #[test]
    fn web_token_is_marked_and_cannot_pick_always() {
        assert!(matches!(
            decider(&who(TokenKind::Web, Scope::Decide)),
            Ok(true)
        ));
        assert!(forbid_web_always(true, false).is_ok());
        assert!(forbid_web_always(true, true).is_err());
    }

    #[test]
    fn others_are_refused() {
        assert!(decider(&who(TokenKind::Web, Scope::Chat)).is_err());
        assert!(decider(&who(TokenKind::External, Scope::Chat)).is_err());
        for kind in [TokenKind::Agent, TokenKind::Module] {
            assert!(decider(&who(kind, Scope::Decide)).is_err(), "{kind:?}");
        }
    }
}
