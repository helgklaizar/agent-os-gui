//! Tool routes: session bookmarks, quick links (stored in `settings`, `If-Match`) and `tool-results`
//! (api.md sections 5.2.1, 5.8; keys `browser.bookmarks.<session_id>` and `browser.quicklinks`).

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;
use crate::tasks_http::session_exists;
use crate::tools_state::Submit;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/sessions/{id}/bookmarks", Scope::Read),
    r("PUT", "/sessions/{id}/bookmarks", Scope::Chat),
    r("GET", "/browser/quicklinks", Scope::Read),
    r("PUT", "/browser/quicklinks", Scope::Chat),
    r("POST", "/sessions/{id}/tool-results", Scope::Chat),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

const QUICKLINKS: &str = "browser.quicklinks";
/// At most this many links in the list.
const MAX_LINKS: usize = 500;

/// Quick links of the start page until they are changed.
fn default_quicklinks() -> Value {
    json!([
        {"title": "Google", "url": "https://www.google.com/"},
        {"title": "YouTube", "url": "https://www.youtube.com/"},
        {"title": "GitHub", "url": "https://github.com/"},
        {"title": "Wikipedia", "url": "https://www.wikipedia.org/"},
        {"title": "HackerNews", "url": "https://news.ycombinator.com/"},
    ])
}

pub fn bookmarks_key(session: i64) -> String {
    format!("browser.bookmarks.{session}")
}

fn page(list: &Value, rev: i64) -> Reply {
    Reply::json(200, list.clone()).header("etag", format!("\"{rev}\""))
}

fn read(ctx: &Ctx<'_>, key: &str) -> Result<Option<(Value, i64)>, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT value_json, rev FROM settings WHERE key = ?1",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(internal)?;
    Ok(row.map(|(json, rev)| {
        (
            serde_json::from_str(&json).unwrap_or_else(|_| json!([])),
            rev,
        )
    }))
}

/// A link is valid: `title` 1..200 characters, `url` is `http` or `https` without spaces and control characters, up to 2048 characters.
fn link_ok(l: &Value) -> bool {
    let Some(o) = l.as_object() else { return false };
    if o.len() != 2 {
        return false;
    }
    let title = o.get("title").and_then(Value::as_str);
    let url = o.get("url").and_then(Value::as_str);
    let (Some(t), Some(u)) = (title, url) else {
        return false;
    };
    let lower = u.to_ascii_lowercase();
    !t.trim().is_empty()
        && t.chars().count() <= 200
        && u.len() <= 2048
        && (lower.starts_with("http://") || lower.starts_with("https://"))
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn parse_if_match(ctx: &Ctx<'_>) -> Result<i64, ApiErr> {
    let raw = ctx.headers.get("if-match").ok_or_else(|| {
        ApiErr::new(
            428,
            "precondition_required",
            "If-Match header with rev is required",
        )
    })?;
    raw.to_str()
        .ok()
        .and_then(|s| s.trim().trim_matches('"').parse().ok())
        .ok_or_else(|| ApiErr::new(400, "bad_request", "If-Match: integer rev expected"))
}

fn put_list(ctx: &Ctx<'_>, key: &str) -> Result<Reply, ApiErr> {
    let want = parse_if_match(ctx)?;
    let body: Value = serde_json::from_slice(ctx.body)
        .map_err(|_| ApiErr::new(400, "bad_request", "Request body is not valid JSON"))?;
    let Some(list) = body.as_array() else {
        return Err(invalid("body", "array of links expected"));
    };
    if list.len() > MAX_LINKS || !list.iter().all(link_ok) {
        return Err(invalid(
            "body",
            "link: title and url (http or https), at most 500 links",
        ));
    }
    let now = ctx.shared.clock.now_unix();
    let json = body.to_string();
    let saved = ctx
        .shared
        .db
        .write(|w| {
            let tx = w.transaction()?;
            let cur: i64 = tx
                .query_row(
                    "SELECT rev FROM settings WHERE key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            if cur != want {
                return Ok(None);
            }
            tx.execute(
                "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(key) DO UPDATE SET value_json = ?2, rev = ?3, updated_at = ?4",
                params![key, json, cur + 1, now],
            )?;
            tx.commit()?;
            Ok(Some(cur + 1))
        })
        .map_err(internal)?;
    match saved {
        Some(rev) => Ok(page(&body, rev)),
        None => Err(ApiErr::new(
            412,
            "precondition_failed",
            "rev changed: reload the list",
        )),
    }
}

const STATUSES: [&str; 5] = ["ok", "error", "timeout", "cancelled", "denied"];
const KNOWN: [&str; 11] = [
    "request_id",
    "kind",
    "status",
    "output",
    "truncated",
    "output_bytes",
    "content_type",
    "url",
    "http_status",
    "error",
    "duration_ms",
];

fn tool_result(ctx: &Ctx<'_>, session: i64) -> Result<Reply, ApiErr> {
    let body: Map<String, Value> = object(ctx)?;
    if body.keys().any(|k| !KNOWN.contains(&k.as_str())) {
        return Err(invalid("body", "unknown field"));
    }
    let rid = body
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 200)
        .ok_or_else(|| invalid("request_id", "string up to 200 characters"))?;
    let kind = match body.get("kind").and_then(Value::as_str) {
        Some(k @ ("browser" | "user")) => k,
        _ => return Err(invalid("kind", "browser or user")),
    };
    if !body
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| STATUSES.contains(&s))
    {
        return Err(invalid("status", "ok, error, timeout, cancelled or denied"));
    }
    session_exists(ctx, session)?;
    match ctx
        .shared
        .tools
        .submit(session, rid, kind, &Value::Object(body.clone()))
    {
        Submit::Accepted => Ok(Reply::json(
            202,
            json!({ "request_id": rid, "status": "accepted" }),
        )),
        Submit::Unknown => Err(not_found()),
        Submit::KindMismatch => Err(invalid("kind", "does not match the command kind")),
        Submit::Closed => Err(ApiErr::new(409, "conflict", "Command closed")
            .details(json!({ "reason": "request_closed" }))),
        Submit::Conflict => Err(ApiErr::new(409, "conflict", "Result already accepted")
            .details(json!({ "reason": "result_exists" }))),
    }
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/sessions/{id}/bookmarks") => {
            let id = id.ok_or_else(not_found)?;
            session_exists(ctx, id)?;
            let (list, rev) = read(ctx, &bookmarks_key(id))?.unwrap_or((json!([]), 0));
            Ok(page(&list, rev))
        }
        ("PUT", "/sessions/{id}/bookmarks") => {
            let id = id.ok_or_else(not_found)?;
            session_exists(ctx, id)?;
            put_list(ctx, &bookmarks_key(id))
        }
        ("GET", "/browser/quicklinks") => {
            let (list, rev) = read(ctx, QUICKLINKS)?.unwrap_or((default_quicklinks(), 0));
            Ok(page(&list, rev))
        }
        ("PUT", "/browser/quicklinks") => put_list(ctx, QUICKLINKS),
        ("POST", "/sessions/{id}/tool-results") => tool_result(ctx, id.ok_or_else(not_found)?),
        _ => Err(not_found()),
    }
}
