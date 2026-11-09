//! Routes `/triggers` (api.md section 5.8), `POST /modules/{module_id}/token` (section 5.6) and `GET /metrics`.
//! Bodies and errors follow `openapi-draft.yaml`; trigger field checks follow the core trigger parser.

use std::collections::BTreeMap;
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::{self, Route};
use crate::store::{Kind, Scope, TokenSpec};

const fn r(method: &'static str, pattern: &'static str, scope: Scope, local_only: bool) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/metrics", Scope::Admin, true),
    r("POST", "/modules/{module_id}/token", Scope::Admin, false),
    r("GET", "/triggers", Scope::Read, false),
    r("POST", "/triggers", Scope::Admin, false),
    r("PATCH", "/triggers/{id}", Scope::Admin, false),
    r("DELETE", "/triggers/{id}", Scope::Admin, false),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

/// Lifetime of a module panel token, in days (as in the core).
const PANEL_TOKEN_DAYS: u64 = 1;
const MAX_SOURCE: usize = 16;
const MAX_MATCH_CHARS: usize = 200;

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/metrics") => Ok(metrics(ctx)),
        ("POST", "/modules/{module_id}/token") => module_token(ctx),
        ("GET", "/triggers") => list(ctx),
        ("POST", "/triggers") => create(ctx),
        ("PATCH", "/triggers/{id}") => patch(ctx, id.unwrap_or(0)),
        ("DELETE", "/triggers/{id}") => delete(ctx, id.unwrap_or(0)),
        _ => Err(not_found()),
    }
}

// ---------- triggers ----------

fn row_json(x: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let m: String = x.get(2)?;
    Ok(json!({
        "id": x.get::<_, i64>(0)?, "source": x.get::<_, String>(1)?,
        "match": serde_json::from_str::<Value>(&m).unwrap_or_else(|_| json!({})),
        "session_id": x.get::<_, Option<i64>>(3)?, "enabled": x.get::<_, i64>(4)? != 0,
    }))
}

const SELECT: &str = "SELECT id, source, match_json, session_id, enabled FROM triggers";

fn get(ctx: &Ctx<'_>, id: i64) -> Result<Option<Value>, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row_json)
        .optional()
        .map_err(internal)
}

fn list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let rows = conn
        .prepare(&format!("{SELECT} ORDER BY id"))
        .map_err(internal)?
        .query_map([], row_json)
        .and_then(|r| r.collect::<Result<Vec<_>, _>>())
        .map_err(internal)?;
    Ok(Reply::json(200, Value::Array(rows)))
}

fn source_of(v: &Value) -> Result<String, ApiErr> {
    match v.as_str() {
        Some(s)
            if (1..=MAX_SOURCE).contains(&s.len())
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
        {
            Ok(s.to_string())
        }
        _ => Err(invalid(
            "source",
            "channel: 1 to 16 characters a-z, 0-9, \"_\" (tg, wa, mail)",
        )),
    }
}

fn match_of(v: &Value) -> Result<Value, ApiErr> {
    let Some(obj) = v.as_object() else {
        return Err(invalid("match", "object {from?, contains?}"));
    };
    let mut out = Map::new();
    for (k, val) in obj {
        if !matches!(k.as_str(), "from" | "contains") {
            return Err(invalid(&format!("match.{k}"), "unknown field"));
        }
        match val
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_MATCH_CHARS)
        {
            Some(s) => {
                out.insert(k.clone(), Value::String(s.to_string()));
            }
            None => {
                return Err(invalid(
                    &format!("match.{k}"),
                    "non-empty string up to 200 characters",
                ));
            }
        }
    }
    Ok(Value::Object(out))
}

fn session_of(ctx: &Ctx<'_>, v: &Value) -> Result<i64, ApiErr> {
    let id = v
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("session_id", "positive integer"))?;
    crate::tasks_http::session_exists(ctx, id)?;
    Ok(id)
}

fn enabled_of(v: &Value) -> Result<bool, ApiErr> {
    v.as_bool()
        .ok_or_else(|| invalid("enabled", "true or false"))
}

fn no_unknown(body: &Map<String, Value>) -> Result<(), ApiErr> {
    match body
        .keys()
        .find(|k| !["source", "match", "session_id", "enabled"].contains(&k.as_str()))
    {
        Some(k) => Err(invalid(k, "unknown field")),
        None => Ok(()),
    }
}

fn create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body = object(ctx)?;
    no_unknown(&body)?;
    let source = source_of(body.get("source").unwrap_or(&Value::Null))?;
    let matched = match body.get("match") {
        Some(v) => match_of(v)?,
        None => json!({}),
    };
    let session = session_of(ctx, body.get("session_id").unwrap_or(&Value::Null))?;
    let enabled = match body.get("enabled") {
        Some(v) => enabled_of(v)?,
        None => true,
    };
    let id = ctx
        .shared
        .db
        .write(|w| {
            w.execute(
                "INSERT INTO triggers(source, match_json, session_id, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![source, matched.to_string(), session, i64::from(enabled)],
            )?;
            Ok(w.last_insert_rowid())
        })
        .map_err(internal)?;
    get(ctx, id)?
        .map(|t| Reply::json(201, t))
        .ok_or_else(not_found)
}

fn patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let body = object(ctx)?;
    no_unknown(&body)?;
    let cur = get(ctx, id)?.ok_or_else(not_found)?;
    let source = match body.get("source") {
        Some(v) => source_of(v)?,
        None => cur["source"].as_str().unwrap_or_default().to_string(),
    };
    let matched = match body.get("match") {
        Some(v) => match_of(v)?,
        None => cur["match"].clone(),
    };
    let session = match body.get("session_id") {
        Some(v) => Some(session_of(ctx, v)?),
        None => cur["session_id"].as_i64(),
    };
    let enabled = match body.get("enabled") {
        Some(v) => enabled_of(v)?,
        None => cur["enabled"].as_bool().unwrap_or(true),
    };
    ctx.shared
        .db
        .write(|w| {
            w.execute(
                "UPDATE triggers SET source = ?2, match_json = ?3, session_id = ?4, enabled = ?5 WHERE id = ?1",
                params![id, source, matched.to_string(), session, i64::from(enabled)],
            )
        })
        .map_err(internal)?;
    get(ctx, id)?
        .map(|t| Reply::json(200, t))
        .ok_or_else(not_found)
}

fn delete(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let n = ctx
        .shared
        .db
        .write(|w| w.execute("DELETE FROM triggers WHERE id = ?1", params![id]))
        .map_err(internal)?;
    if n == 0 {
        return Err(not_found());
    }
    Ok(Reply::no_content())
}

// ---------- module panel token ----------

fn module_token(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let module = ctx
        .path
        .strip_prefix("/v1/modules/")
        .and_then(|p| p.strip_suffix("/token"))
        .unwrap_or_default();
    let installed = crate::admin::modules_http::installed(ctx.shared)?
        .into_iter()
        .find(|m| m["id"] == module)
        .ok_or_else(not_found)?;
    if installed["enabled"] != true {
        return Err(ApiErr::new(409, "conflict", "Module is disabled")
            .details(json!({ "reason": "module_disabled" })));
    }
    let allow: Vec<String> = installed["manifest"]["permissions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect();
    let spec = TokenSpec {
        label: format!("module:{module}"),
        scope: Scope::Chat,
        kind: Kind::Module,
        ttl: Some(Duration::from_secs(PANEL_TOKEN_DAYS * 86_400)),
        allowlist: Some(allow),
    };
    let now = ctx.shared.clock.now_unix();
    let issued = ctx
        .shared
        .lock()
        .tokens
        .issue(&spec, now, Some(ctx.who.id))
        .map_err(internal)?;
    Ok(Reply::json(
        201,
        json!({ "token": issued.token, "expires_at": now + (PANEL_TOKEN_DAYS * 86_400) as i64 }),
    ))
}

// ---------- metrics ----------

/// Counters without user data: the route is a template, not the actual path.
fn metrics(ctx: &Ctx<'_>) -> Reply {
    let mut counts: BTreeMap<(String, u16), i64> = BTreeMap::new();
    for rec in &ctx.shared.lock().requests {
        let route = rec
            .path
            .strip_prefix("/v1")
            .and_then(|p| routes::find(&rec.method, p))
            .map_or_else(|| "unmatched".to_string(), |(r, _)| r.pattern.to_string());
        *counts.entry((route, rec.status)).or_insert(0) += 1;
    }
    let rows: Vec<Value> = counts
        .into_iter()
        .map(|((route, status), count)| json!({ "route": route, "status": status, "count": count }))
        .collect();
    Reply::json(
        200,
        json!({ "http_requests_total": rows, "ws_connections": ctx.shared.hub.conns() }),
    )
}
