//! `GET /search` and the service indicator frame `/_test/indicator-frame`: route table and handlers.
//!
//! The memory source is taken from the `memory` page of the `settings` table (`PATCH /v1/settings/memory`); the mock has
//! no separate switch.

use serde_json::{Value, json};

use crate::FrameType;
use crate::chat_http::{internal, invalid};
use crate::handlers::Ctx;
use crate::indicators::{IndState, IndicatorStore};
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::search::MemorySource;
use crate::settings_http::row;
use crate::store::Scope;

/// Length limit of the indicator frame `text` (as in `controllers::controllers::indicator::MAX_TEXT_CHARS`).
const MAX_TEXT_CHARS: usize = 500;

const fn r(method: &'static str, pattern: &'static str, scope: Scope, local_only: bool) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/search", Scope::Read, false),
    r("POST", "/_test/indicator-frame", Scope::Admin, true),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/search") => search_get(ctx),
        ("POST", "/_test/indicator-frame") => indicator_frame(ctx),
        _ => Err(ApiErr::new(404, "not_found", "Route not found")),
    }
}

/// Value of a query-string parameter with `%XX` (UTF-8) and `+` decoding; an invalid sequence gives `None`.
pub(crate) fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let raw = query?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == name).then_some(v)
    })?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn memory_source(ctx: &Ctx<'_>) -> Result<MemorySource, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let Some((text, _)) = row(&conn, "memory").map_err(internal)? else {
        return Ok(MemorySource::Local);
    };
    let doc: Value = serde_json::from_str(&text).map_err(internal)?;
    Ok(doc
        .get("source")
        .and_then(Value::as_str)
        .and_then(MemorySource::parse)
        .unwrap_or(MemorySource::Local))
}

fn search_get(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let q = query_param(ctx.query, "q")
        .ok_or_else(|| invalid("q", "q is required and must be a UTF-8 string"))?;
    let limit = match query_param(ctx.query, "limit") {
        None => 20,
        Some(l) => l
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(|| invalid("limit", "limit: 1 to 100"))?,
    };
    let source = memory_source(ctx)?;
    let items = ctx
        .shared
        .search
        .search(&ctx.shared.db, source, &q, limit)
        .map_err(internal)?;
    Ok(Reply::json(200, json!({ "items": items })))
}

fn indicator_frame(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body: Value = serde_json::from_slice(ctx.body)
        .map_err(|_| ApiErr::new(400, "bad_request", "Request body is not valid JSON"))?;
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| (1..=64).contains(&s.len()))
        .ok_or_else(|| invalid("id", "id: 1 to 64 characters"))?;
    let state = body
        .get("state")
        .and_then(Value::as_str)
        .and_then(IndState::parse)
        .ok_or_else(|| invalid("state", "state: ok, warn, bad, nodata or unknown"))?;
    let value = match body.get("value") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid("value", "value: finite number"))?,
        ),
    };
    let text = match body.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.chars().count() <= MAX_TEXT_CHARS => Some(t.clone()),
        Some(_) => return Err(invalid("text", "text: string up to 500 characters")),
    };
    let now = ctx.shared.clock.now_unix();
    let t = IndicatorStore::new(ctx.shared.db.clone())
        .apply(id, state, now)
        .map_err(internal)?;
    // The transition event goes to `events` subscribers like any other (events_ops::add).
    if let Some(event_id) = t.event_id {
        let conn = ctx.shared.db.reader().map_err(internal)?;
        if let Some(event) = crate::events_ops::get(&conn, event_id).map_err(internal)? {
            ctx.shared.hub.emit_keep("events", FrameType::Event, event);
        }
    }
    let mut frame =
        json!({ "id": id, "state": state.as_str(), "transition_seq": t.seq, "ts": now });
    if let (Some(v), Some(o)) = (value, frame.as_object_mut()) {
        o.insert("value".into(), json!(v));
    }
    if let (Some(t), Some(o)) = (text, frame.as_object_mut()) {
        o.insert("text".into(), json!(t));
    }
    ctx.shared
        .hub
        .emit("bar.indicators", FrameType::Delta, frame);
    Ok(Reply::json(
        200,
        json!({ "changed": t.changed, "seq": t.seq, "event_id": t.event_id }),
    ))
}
