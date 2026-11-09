//! Route handlers: theme with `If-Match`, tokens, service `echo` and `advance`.

use std::time::Duration;

use hyper::HeaderMap;
use serde_json::{Value, json};

use crate::auth::Authed;
use crate::hub::Out;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::settings_store::ThemeWrite;
use crate::state::Shared;
use crate::store::{Kind, Scope, StoreError, TokenRow, TokenSpec};
use crate::{EchoRecord, FrameType};

pub struct Ctx<'a> {
    pub shared: &'a std::sync::Arc<Shared>,
    /// Whole request path, with `/v1`.
    pub path: &'a str,
    pub who: &'a Authed,
    pub headers: &'a HeaderMap,
    pub body: &'a [u8],
    pub query: Option<&'a str>,
}

fn json_body(ctx: &Ctx<'_>) -> Result<Value, ApiErr> {
    serde_json::from_slice(ctx.body)
        .map_err(|_| ApiErr::new(400, "bad_request", "Request body is not valid JSON"))
}

fn invalid(fields: &[&str], message: &str) -> ApiErr {
    ApiErr::new(422, "validation_failed", message).details(json!({ "fields": fields }))
}

/// Storage failure: the client gets `500 internal` without details, the error text goes to the log.
fn internal(e: StoreError) -> ApiErr {
    tracing::error!(error = %e, "fake-agentd storage failure");
    ApiErr::new(500, "internal", "Storage failure")
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    if !crate::fail_inject::owns(route)
        && let Some(status) = ctx
            .shared
            .fail
            .take(&format!("{} {}", route.method, ctx.path))
    {
        return Err(crate::fail_inject::error_for(status));
    }
    match (route.method, route.pattern) {
        ("GET", "/settings/theme") => theme_get(ctx),
        ("PATCH", "/settings/theme") => theme_patch(ctx),
        ("GET", "/tokens") => tokens_list(ctx),
        ("POST", "/tokens") => tokens_issue(ctx),
        ("DELETE", "/tokens/{id}") => token_revoke(ctx, id),
        ("POST", "/_test/echo") => echo(ctx),
        ("POST", "/_test/advance") => advance(ctx),
        _ if crate::bar_http::ROUTES
            .iter()
            .any(|r| r.method == route.method && r.pattern == route.pattern) =>
        {
            crate::bar_http::dispatch(route, id, ctx)
        }
        _ if crate::chat_http::owns(route) => crate::chat_http::dispatch(route, id, ctx),
        _ if crate::events_http::owns(route) => crate::events_http::dispatch(route, id, ctx),
        _ if crate::perm_http::owns(route) => crate::perm_http::dispatch(route, id, ctx),
        _ if crate::sess_http::owns(route) => crate::sess_http::dispatch(route, id, ctx),
        _ if crate::ops_http::owns(route) => crate::ops_http::dispatch(route, id, ctx),
        _ if crate::tasks_http::owns(route) => crate::tasks_http::dispatch(route, id, ctx),
        _ if crate::tools_http::owns(route) => crate::tools_http::dispatch(route, id, ctx),
        _ if crate::fail_inject::owns(route) => crate::fail_inject::dispatch(route, ctx),
        _ if crate::settings_http::owns(route) => crate::settings_http::dispatch(route, ctx),
        _ if crate::secrets_http::owns(route) => crate::secrets_http::dispatch(route, ctx),
        _ if crate::people_http::owns(route) => crate::people_http::dispatch(route, id, ctx),
        _ if crate::modules_http::owns(route) => crate::modules_http::dispatch(route, ctx),
        _ if crate::tunnels_http::owns(route) => crate::tunnels_http::dispatch(route, id, ctx),
        _ if crate::files_http::owns(route) => crate::files_http::dispatch(route, ctx),
        _ if crate::search_http::owns(route) => crate::search_http::dispatch(route, ctx),
        _ => crate::tree_http::dispatch(route, id, ctx),
    }
}

fn theme_page(theme: &str, rev: i64) -> Reply {
    Reply::json(200, json!({ "rev": rev, "theme": theme })).header("etag", format!("\"{rev}\""))
}

fn theme_get(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let (theme, rev) = ctx.shared.lock().settings.theme().map_err(internal)?;
    Ok(theme_page(&theme, rev))
}

fn theme_patch(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let raw = ctx.headers.get("if-match").ok_or_else(|| {
        ApiErr::new(
            428,
            "precondition_required",
            "If-Match header with rev is required",
        )
    })?;
    let want: i64 = raw
        .to_str()
        .ok()
        .and_then(|s| s.trim().trim_matches('"').parse().ok())
        .ok_or_else(|| ApiErr::new(400, "bad_request", "If-Match: integer rev expected"))?;
    let body = json_body(ctx)?;
    let theme = match body.get("theme").and_then(Value::as_str) {
        Some(t @ ("auto" | "dark" | "light")) => t.to_string(),
        _ => return Err(invalid(&["theme"], "theme: auto, dark or light")),
    };
    let now = ctx.shared.clock.now_unix();
    let written = ctx
        .shared
        .lock()
        .settings
        .set_theme(want, &theme, now)
        .map_err(internal)?;
    let ThemeWrite::Saved(rev) = written else {
        return Err(ApiErr::new(
            412,
            "precondition_failed",
            "rev changed: reload the page",
        ));
    };
    ctx.shared.hub.emit(
        "settings",
        FrameType::Delta,
        json!({ "page": "theme", "rev": rev }),
    );
    Ok(theme_page(&theme, rev))
}

fn token_json(r: &TokenRow) -> Value {
    json!({
        "id": r.id, "name": r.label, "kind": r.kind.as_str(), "scope": r.scope.as_str(),
        "allowlist": r.allowlist, "created_by": r.created_by, "created_at": r.created_at,
        "expires_at": r.expires_at, "last_used": r.last_used_at, "revoked": r.revoked_at.is_some(),
    })
}

fn tokens_list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let rows = ctx.shared.lock().tokens.rows().map_err(internal)?;
    Ok(Reply::json(
        200,
        Value::Array(rows.iter().map(token_json).collect()),
    ))
}

fn tokens_issue(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body = json_body(ctx)?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(&["name"], "name is required"))?;
    let scope = body
        .get("scope")
        .and_then(Value::as_str)
        .and_then(Scope::parse)
        .ok_or_else(|| invalid(&["scope"], "scope: read, chat or admin"))?;
    let kind = match body.get("kind") {
        None | Some(Value::Null) => Kind::External,
        Some(k) => k
            .as_str()
            .and_then(Kind::parse)
            .ok_or_else(|| invalid(&["kind"], "kind: gui, external, test, agent or module"))?,
    };
    let ttl_days = match body.get("ttl_days") {
        None | Some(Value::Null) => (kind == Kind::External).then_some(30),
        Some(v) => Some(
            v.as_u64()
                .filter(|d| (1..=90).contains(d))
                .ok_or_else(|| invalid(&["ttl_days"], "ttl_days: 1 to 90"))?,
        ),
    };
    let allowlist = match body.get("allowlist") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_array()
                .and_then(|a| {
                    a.iter()
                        .map(|m| m.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| invalid(&["allowlist"], "allowlist: list of strings"))?,
        ),
    };
    let spec = TokenSpec {
        label: name.to_string(),
        scope,
        kind,
        ttl: ttl_days.map(|d| Duration::from_secs(d * 86_400)),
        allowlist,
    };
    let now = ctx.shared.clock.now_unix();
    let inner = ctx.shared.lock();
    let issued = inner
        .tokens
        .issue(&spec, now, Some(ctx.who.id))
        .map_err(|e| match e {
            StoreError::Invalid(m) => invalid(&["token"], &m),
            other => internal(other),
        })?;
    let rows = inner.tokens.rows().map_err(internal)?;
    let mut out = rows
        .iter()
        .find(|r| r.id == issued.id)
        .map(token_json)
        .unwrap_or(Value::Null);
    out["token"] = Value::String(issued.token);
    Ok(Reply::json(201, out))
}

fn token_revoke(ctx: &Ctx<'_>, id: Option<i64>) -> Result<Reply, ApiErr> {
    let id = id.ok_or_else(|| ApiErr::new(404, "not_found", "Token not found"))?;
    let now = ctx.shared.clock.now_unix();
    let found = ctx.shared.lock().tokens.revoke(id, now).map_err(internal)?;
    if !found {
        return Err(ApiErr::new(404, "not_found", "Token not found"));
    }
    ctx.shared.hub.control(Out::Revoke(id));
    Ok(Reply::no_content())
}

fn echo(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body = json_body(ctx)?;
    let client_id = body
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| (1..=128).contains(&s.len()))
        .ok_or_else(|| invalid(&["client_id"], "client_id: 1 to 128 characters"))?;
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(&["text"], "text must not be empty"))?;
    let mut inner = ctx.shared.lock();
    if let Some(old) = inner.echo.iter().find(|e| e.client_id == client_id) {
        if old.text != text {
            return Err(ApiErr::new(
                409,
                "client_id_conflict",
                "client_id is already taken by a different text",
            ));
        }
        return Ok(Reply::json(
            200,
            json!({ "client_id": client_id, "text": text, "ordinal": old.ordinal, "duplicate": true }),
        ));
    }
    let ordinal = inner.echo.len() as u64 + 1;
    inner.echo.push(EchoRecord {
        client_id: client_id.to_string(),
        text: text.to_string(),
        ordinal,
    });
    Ok(Reply::json(
        200,
        json!({ "client_id": client_id, "text": text, "ordinal": ordinal, "duplicate": false }),
    ))
}

fn advance(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body = json_body(ctx)?;
    let ms = body
        .get("ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(&["ms"], "ms: milliseconds"))?;
    ctx.shared.clock.advance(Duration::from_millis(ms));
    crate::perm_sweep::sweep_expired(ctx.shared);
    Ok(Reply::json(
        200,
        json!({ "now_unix": ctx.shared.clock.now_unix() }),
    ))
}
