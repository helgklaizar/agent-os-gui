//! Applying `ask` to HTTP mutations (docs/reference/api.md 3, A4-03). Assumption A4-12: `202` and a permission request apply to mutations by `agent`
//! and `module` tokens; for `gui` and `external` confirmation is a client dialog and they never get `202`. The layer sits
//! after token verification and before handlers; the handlers themselves know nothing about permissions.

use std::sync::Arc;

use agentd_core::{Action, Actor, Gate, GateInput, Principal, Subject, TokenKind};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::access::MatchedRoute;
use crate::error::ApiError;
use crate::state::ApiState;

/// Which action the mutation performs: action, session and target for the log and the card (without the request body: secret
/// values and message texts do not reach the log).
struct Mutation {
    action: Action,
    session_id: Option<i64>,
    target: String,
}

/// Value of the template substitution `{name}` in the path.
fn param(template: &str, path: &str, name: &str) -> Option<String> {
    let pat = format!("{{{name}}}");
    template
        .trim_start_matches('/')
        .split('/')
        .zip(path.trim_start_matches('/').split('/'))
        .find(|(t, _)| *t == pat)
        .map(|(_, p)| p.to_string())
}

/// Value of the `path` query parameter as is (for the log target; the handler parses the path).
fn query_path(query: Option<&str>) -> String {
    query
        .unwrap_or_default()
        .split('&')
        .find_map(|p| p.strip_prefix("path="))
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

fn classify(method: &str, template: &str, path: &str, query: Option<&str>) -> Option<Mutation> {
    let session = || {
        param(template, path, "id")
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| *n > 0)
    };
    let m = match (method, template) {
        ("DELETE", "/sessions/{id}") => Mutation {
            action: Action::Delete,
            session_id: session(),
            target: format!("session:{}", param(template, path, "id")?),
        },
        ("POST", "/sessions/{id}/clear") => Mutation {
            action: Action::Delete,
            session_id: session(),
            target: format!("session:{}:messages", param(template, path, "id")?),
        },
        ("POST", "/sessions/{id}/messages") => Mutation {
            action: Action::Send,
            session_id: session(),
            target: format!("session:{}", param(template, path, "id")?),
        },
        ("DELETE", "/folders/{id}") => Mutation {
            action: Action::Delete,
            session_id: None,
            target: format!("folder:{}", param(template, path, "id")?),
        },
        ("PATCH", "/settings/{page}") => Mutation {
            action: Action::ConfigWrite,
            session_id: None,
            target: format!("settings:{}", param(template, path, "page")?),
        },
        ("PATCH", "/settings/theme" | "/settings/indicators") => Mutation {
            action: Action::ConfigWrite,
            session_id: None,
            target: format!("settings:{}", template.rsplit('/').next()?),
        },
        ("PUT" | "DELETE", "/secrets/{ref}") => Mutation {
            action: Action::ConfigWrite,
            session_id: None,
            target: format!("secret:{}", param(template, path, "ref")?),
        },
        ("PUT", "/files/content") => {
            let file = query_path(query);
            Mutation {
                // `AGENT.md` configures agent behaviour, `memory/` is memory.
                action: if file == "AGENT.md" {
                    Action::ConfigWrite
                } else {
                    Action::MemoryWrite
                },
                session_id: None,
                target: format!("file:{file}"),
            }
        }
        _ => return None,
    };
    Some(m)
}

pub(crate) async fn layer(
    State(state): State<Arc<ApiState>>,
    req: Request,
    next: Next,
) -> Response {
    let (Some(who), Some(MatchedRoute(route))) = (
        req.extensions().get::<Principal>().cloned(),
        req.extensions().get::<MatchedRoute>().copied(),
    ) else {
        return next.run(req).await;
    };
    let actor = match who.kind {
        TokenKind::Agent => Actor::Agent,
        TokenKind::Module => Actor::Module,
        _ => return next.run(req).await,
    };
    // Table templates are without the `/v1` prefix.
    let path = req
        .uri()
        .path()
        .strip_prefix("/v1")
        .unwrap_or(req.uri().path());
    let Some(m) = classify(route.method, route.template, path, req.uri().query()) else {
        return next.run(req).await;
    };
    let services = state.services();
    let (Some(gate), Some(engine)) = (
        services.as_ref().and_then(|s| s.gate.clone()),
        services.as_ref().and_then(|s| s.engine.clone()),
    ) else {
        return ApiError::core_down("Permission gate unavailable").into_response();
    };
    // No such session: the handler's ordinary `404`, not a permission request for a nonexistent object.
    let mut persona_id = None;
    if let Some(sid) = m.session_id {
        match engine
            .store()
            .scope_exists(agentd_core::Level::Session, sid)
            .await
        {
            Ok(true) => {}
            Ok(false) => return next.run(req).await,
            Err(_) => return ApiError::core_down("Permission store unavailable").into_response(),
        }
        persona_id = engine.store().session_persona(sid).await.ok().flatten();
    }
    let input = GateInput {
        subject: Subject {
            actor,
            session_id: m.session_id,
            persona_id,
        },
        action: m.action,
        target: Some(m.target),
        args: Some(json!({"route": format!("{} {}", route.method, route.template)})),
        untrusted: None,
        grant_token: Some(who.token_id),
    };
    match gate.check(input).await {
        Ok(Gate::Allow { .. }) => next.run(req).await,
        Ok(Gate::Ask(r)) => {
            (StatusCode::ACCEPTED, axum::Json::<Value>(r.to_json())).into_response()
        }
        Ok(Gate::Deny { reason, .. }) => ApiError::permission_denied(reason).into_response(),
        Err(_) => ApiError::core_down("Permission store unavailable").into_response(),
    }
}
