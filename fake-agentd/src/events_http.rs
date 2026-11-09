//! Event routes (api.md section 5.3): feed, mark as read, subtype registry.

use serde_json::{Value, json};

use crate::events_ops::{self as ops, Filter};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/events", Scope::Read),
    r("POST", "/events/read", Scope::Chat),
    r("GET", "/events/subtypes", Scope::Read),
    r("POST", "/events/{id}/read", Scope::Chat),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

fn internal(e: impl std::fmt::Display) -> ApiErr {
    tracing::error!(error = %e, "fake-agentd storage failure");
    ApiErr::new(500, "internal", "Storage failure")
}

fn bad(why: &str) -> ApiErr {
    ApiErr::new(400, "bad_request", why)
}

const KINDS: [&str; 3] = ["ai", "task", "msg"];

fn kind(v: &str) -> Result<String, ApiErr> {
    KINDS
        .contains(&v)
        .then(|| v.to_string())
        .ok_or_else(|| bad("kind: ai, task or msg"))
}

fn int(v: &str, key: &str) -> Result<i64, ApiErr> {
    v.parse()
        .map_err(|_| bad(&format!("{key}: integer expected")))
}

/// Percent-decoding of a query value (a subtype may be non-ASCII).
fn decode(raw: &str) -> Result<String, ApiErr> {
    let mut out = Vec::with_capacity(raw.len());
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = raw
                    .get(i + 1..i + 3)
                    .ok_or_else(|| bad("sub: invalid encoding"))?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| bad("sub: invalid encoding"))?);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| bad("sub: not UTF-8"))
}

/// `?kind&sub&unread&session_id&before&limit`; an unparseable value is `400`.
fn list_filter(query: Option<&str>) -> Result<Filter, ApiErr> {
    let mut f = Filter::default();
    for (k, v) in query
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
    {
        match k {
            "kind" => f.kind = Some(kind(v)?),
            "sub" => f.sub = Some(decode(v)?),
            "unread" => {
                f.unread = Some(match v {
                    "true" => true,
                    "false" => false,
                    _ => return Err(bad("unread: true or false")),
                })
            }
            "session_id" => f.session_id = Some(int(v, k)?),
            "before" => f.before = Some(int(v, k)?),
            "limit" => {
                let n = int(v, k)?;
                if !(1..=200).contains(&n) {
                    return Err(bad("limit: 1 to 200"));
                }
                f.limit = Some(n);
            }
            _ => {}
        }
    }
    Ok(f)
}

/// Body of `POST /events/read`: `ids`, `kind`, `sub`, `session_id`; an empty body means all unread.
fn read_filter(body: &[u8]) -> Result<Filter, ApiErr> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Filter::default());
    }
    let Ok(Value::Object(m)) = serde_json::from_slice::<Value>(body) else {
        return Err(bad("Request body: JSON object expected"));
    };
    let mut f = Filter::default();
    if let Some(v) = m.get("kind") {
        f.kind = Some(kind(
            v.as_str().ok_or_else(|| bad("kind: string expected"))?,
        )?);
    }
    if let Some(v) = m.get("sub") {
        f.sub = Some(
            v.as_str()
                .ok_or_else(|| bad("sub: string expected"))?
                .to_string(),
        );
    }
    if let Some(v) = m.get("session_id") {
        f.session_id = Some(
            v.as_i64()
                .ok_or_else(|| bad("session_id: integer expected"))?,
        );
    }
    if let Some(v) = m.get("ids") {
        let list = v.as_array().ok_or_else(|| bad("ids: array expected"))?;
        let ids = list
            .iter()
            .map(|i| i.as_i64().ok_or_else(|| bad("ids: integers expected")))
            .collect::<Result<_, _>>()?;
        f.ids = Some(ids);
    }
    Ok(f)
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let none = || ApiErr::new(404, "not_found", "Object not found");
    match (route.method, route.pattern) {
        ("GET", "/events") => {
            let f = list_filter(ctx.query)?;
            let conn = ctx.shared.db.reader().map_err(internal)?;
            Ok(Reply::json(200, ops::list(&conn, &f).map_err(internal)?))
        }
        ("POST", "/events/read") => {
            let f = read_filter(ctx.body)?;
            let ids = ops::mark_read(ctx.shared, &f).map_err(internal)?;
            Ok(Reply::json(200, json!({ "marked": ids.len() })))
        }
        ("GET", "/events/subtypes") => {
            let items: Vec<Value> = ctx
                .shared
                .registry
                .list()
                .iter()
                .map(|s| s.json())
                .collect();
            Ok(Reply::json(200, Value::Array(items)))
        }
        ("POST", "/events/{id}/read") => {
            let id = id.ok_or_else(none)?;
            let exists = {
                let conn = ctx.shared.db.reader().map_err(internal)?;
                ops::get(&conn, id).map_err(internal)?.is_some()
            };
            if !exists {
                return Err(none());
            }
            let f = Filter {
                ids: Some(vec![id]),
                ..Filter::default()
            };
            ops::mark_read(ctx.shared, &f).map_err(internal)?;
            Ok(Reply::no_content())
        }
        _ => Err(none()),
    }
}
