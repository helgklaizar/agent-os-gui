//! `GET /v1/actions` (docs/reference/api.md 5.4): the action log in pages, newest first. Arguments and rollback are not returned.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{ActionQuery, ActionRow, FieldError};
use axum::extract::{RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, ok};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const ACTORS: [&str; 5] = ["agent", "cron", "user", "observer", "module"];

fn pairs(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (p.to_string(), String::new()),
        })
        .collect()
}

fn num(q: &HashMap<String, String>, key: &str, errs: &mut Vec<FieldError>) -> Option<i64> {
    let v = q.get(key)?;
    match v.parse::<i64>() {
        Ok(n) if n >= 0 => Some(n),
        _ => {
            errs.push(FieldError::new(key, "non-negative integer"));
            None
        }
    }
}

/// Query string parsing; the resulting `limit` is one greater, to learn whether a next page exists.
pub(crate) fn parse(raw: Option<String>) -> Result<(ActionQuery, usize), Vec<FieldError>> {
    let q = pairs(raw);
    let mut errs = Vec::new();
    let session_id = num(&q, "session_id", &mut errs);
    let from = num(&q, "from", &mut errs);
    let to = num(&q, "to", &mut errs);
    let before = num(&q, "cursor", &mut errs);
    let actor = q.get("actor").cloned();
    if actor.as_deref().is_some_and(|a| !ACTORS.contains(&a)) {
        errs.push(FieldError::new(
            "actor",
            "agent, cron, user, observer or module",
        ));
    }
    let limit = match q.get("limit") {
        None => DEFAULT_LIMIT,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => {
                errs.push(FieldError::new("limit", "integer from 1 to 200"));
                DEFAULT_LIMIT
            }
        },
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok((
        ActionQuery {
            session_id,
            actor,
            from,
            to,
            before,
            limit: limit + 1,
        },
        limit,
    ))
}

fn item(r: &ActionRow) -> Value {
    json!({
        "id": r.id, "ts": r.ts, "actor": r.actor, "session_id": r.session_id,
        "persona_id": r.persona_id, "action": r.action, "target": r.target,
        "decision": r.decision.map(agentd_core::Decision::as_str), "result": r.result,
    })
}

async fn list(
    State(state): State<Arc<ApiState>>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let (query, limit) = parse(raw).map_err(invalid)?;
    let store = state
        .services()
        .and_then(|s| s.audit.clone())
        .ok_or_else(|| ApiError::core_down("Action log unavailable"))?;
    let mut rows = store
        .store()
        .list_actions(query)
        .await
        .map_err(|_| ApiError::core_down("Action log unavailable"))?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = if more {
        rows.last().map(|r| r.id.to_string())
    } else {
        None
    };
    Ok(ok(
        json!({"items": rows.iter().map(item).collect::<Vec<_>>(), "next_cursor": next}),
    ))
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    get(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<(ActionQuery, usize), Vec<FieldError>> {
        parse(Some(s.to_string()))
    }

    #[test]
    fn defaults_and_limit_plus_one() {
        let (q, l) = p("").unwrap();
        assert_eq!((l, q.limit), (50, 51));
        let (q, l) = p("limit=200&actor=cron&session_id=3&from=1&to=9&cursor=7").unwrap();
        assert_eq!((l, q.limit, q.actor.as_deref()), (200, 201, Some("cron")));
        assert_eq!(
            (q.session_id, q.from, q.to, q.before),
            (Some(3), Some(1), Some(9), Some(7))
        );
    }

    #[test]
    fn each_bad_field_is_named() {
        let e = p("limit=201&actor=x&cursor=-1&from=a&to=&session_id=1.5").unwrap_err();
        let names: Vec<&str> = e.iter().map(|f| f.field.as_str()).collect();
        for n in ["limit", "actor", "cursor", "from", "to", "session_id"] {
            assert!(names.contains(&n), "{n} {names:?}");
        }
        assert!(p("limit=0").is_err());
    }
}
