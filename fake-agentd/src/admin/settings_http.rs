//! Settings pages `GET`/`PATCH /v1/settings/{page}` and `/v1/settings/indicators` (api.md section 5.7).
//! A page document is a `settings` row keyed by the page name; no row means the default document and `rev = 0`.
//! Write: `If-Match` check, merge (RFC 7386), validation and save - one transaction of the single writer.

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::FrameType;
use crate::chat_http::{internal, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::settings_conn::FieldErr;
use crate::settings_pages as pages;
use crate::store::Scope;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

/// The exact `/settings/indicators` comes before the `/settings/{page}` template.
pub const ROUTES: &[Route] = &[
    r("GET", "/settings/indicators", Scope::Read),
    r("PATCH", "/settings/indicators", Scope::Admin),
    r("GET", "/settings/{page}", Scope::Read),
    r("PATCH", "/settings/{page}", Scope::Admin),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

pub fn page_reply(mut doc: Value, rev: i64) -> Reply {
    if let Value::Object(o) = &mut doc {
        o.insert("rev".into(), rev.into());
    }
    Reply::json(200, doc).header("etag", format!("\"{rev}\""))
}

/// `If-Match`: no header - 428, not an integer - 400.
pub fn if_match(ctx: &Ctx<'_>) -> Result<i64, ApiErr> {
    let raw = ctx.headers.get("if-match").ok_or_else(|| {
        ApiErr::new(
            428,
            "precondition_required",
            "If-Match header with rev is required",
        )
    })?;
    raw.to_str()
        .ok()
        .map(|s| s.trim().trim_start_matches("W/").trim_matches('"'))
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .ok_or_else(|| ApiErr::new(400, "bad_request", "If-Match: integer rev expected"))
}

pub fn stale() -> ApiErr {
    ApiErr::new(412, "precondition_failed", "rev changed: reload the page")
}

/// 422 with field paths (the refusal text has no values).
pub fn invalid_fields(errs: &[FieldErr]) -> ApiErr {
    let fields: Vec<&str> = errs.iter().map(|(f, _)| f.as_str()).collect();
    let message = errs.first().map_or("Fields failed validation", |(_, m)| m);
    ApiErr::new(422, "validation_failed", message).details(json!({ "fields": fields }))
}

/// Publishes a `settings` frame about a page write.
pub fn emit(ctx: &Ctx<'_>, page: &str, rev: i64) {
    ctx.shared.hub.emit(
        "settings",
        FrameType::Delta,
        json!({ "page": page, "rev": rev }),
    );
}

pub(crate) fn row(
    conn: &rusqlite::Connection,
    key: &str,
) -> rusqlite::Result<Option<(String, i64)>> {
    conn.query_row(
        "SELECT value_json, rev FROM settings WHERE key = ?1",
        params![key],
        |x| Ok((x.get(0)?, x.get(1)?)),
    )
    .optional()
}

/// Page document and `rev` (default `rev = 0`).
pub fn read_page(ctx: &Ctx<'_>, page: &str) -> Result<(Value, i64), ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    Ok(match row(&conn, page).map_err(internal)? {
        None => (pages::default_doc(page), 0),
        Some((text, rev)) => (serde_json::from_str(&text).map_err(internal)?, rev),
    })
}

fn computed(ctx: &Ctx<'_>, page: &str) -> Result<Value, ApiErr> {
    if page == "about" {
        let conn = ctx.shared.db.reader().map_err(internal)?;
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |x| x.get(0))
            .map_err(internal)?;
        return Ok(json!({
            "core_version": crate::reply::VERSION,
            "api": { "major": ctx.shared.api_major, "minor": 0 },
            "db_version": v,
            "capabilities": ["settings", "secrets", "files", "modules", "tunnels"],
        }));
    }
    let ptrace = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()
        .and_then(|t| t.trim().parse::<i64>().ok());
    Ok(json!({
        "ptrace_scope": ptrace,
        "disk_encryption": "unknown",
        "secrets_present": !ctx.shared.secrets.list().is_empty(),
    }))
}

fn page_of<'a>(ctx: &'a Ctx<'_>) -> &'a str {
    ctx.path.strip_prefix("/v1/settings/").unwrap_or("")
}

pub(crate) enum Outcome {
    Saved(Value, i64),
    Stale,
    Invalid(Vec<FieldErr>),
}

fn get(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let page = page_of(ctx);
    if !pages::is_known(page) {
        return Err(not_found());
    }
    if pages::is_read_only(page) {
        return Ok(page_reply(computed(ctx, page)?, 0));
    }
    let (doc, rev) = read_page(ctx, page)?;
    Ok(page_reply(doc, rev))
}

fn patch(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let page = page_of(ctx);
    if !pages::is_known(page) {
        return Err(not_found());
    }
    let want = if_match(ctx)?;
    if pages::is_read_only(page) {
        return Err(ApiErr::new(403, "permission_denied", "Page is read-only")
            .details(json!({ "reason": "read_only" })));
    }
    let mut body = object(ctx)?;
    body.remove("rev");
    let patch = Value::Object(body);
    let now = ctx.shared.clock.now_unix();
    let out = ctx
        .shared
        .db
        .write(|tx| {
            let (current, rev) = match row(tx, page)? {
                None => (pages::default_doc(page), 0),
                Some((t, rev)) => (serde_json::from_str(&t).unwrap_or(Value::Null), rev),
            };
            if rev != want {
                return Ok(Outcome::Stale);
            }
            let mut merged = pages::merge_patch(&current, &patch);
            if let Value::Object(o) = &mut merged {
                o.remove("rev");
            }
            let errs = pages::validate(page, &merged);
            if !errs.is_empty() {
                return Ok(Outcome::Invalid(errs));
            }
            tx.execute(
                "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, ?2, 1, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, rev = settings.rev + 1, updated_at = excluded.updated_at",
                params![page, merged.to_string(), now],
            )?;
            Ok(Outcome::Saved(merged, rev + 1))
        })
        .map_err(internal)?;
    match out {
        Outcome::Stale => Err(stale()),
        Outcome::Invalid(e) => Err(invalid_fields(&e)),
        Outcome::Saved(doc, rev) => {
            emit(ctx, page, rev);
            Ok(page_reply(doc, rev))
        }
    }
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/settings/indicators") => crate::settings_indicators::doc(ctx),
        ("PATCH", "/settings/indicators") => crate::settings_indicators::patch(ctx),
        ("GET", _) => get(ctx),
        _ => patch(ctx),
    }
}
