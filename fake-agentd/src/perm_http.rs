//! Permission routes: matrix, `PUT` rule, requests and decisions, log and findings (api.md section 5.4).
//!
//! Only the local GUI decides a request: `admin` and `kind` `gui` or `test`. Log responses: `{items, next_cursor}`.

use serde_json::{Value, json};

use crate::chat_http::{int_param, internal, invalid, not_found, object, query_pairs};
use crate::handlers::Ctx;
use crate::perm_flow::{self, Outcome, ResolveErr};
use crate::perm_store::{self as store, Mode, PutError};
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::{Kind, Scope};

const fn r(method: &'static str, pattern: &'static str, scope: Scope, local_only: bool) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only,
    }
}

pub const PUT_ROUTE: Route = r(
    "PUT",
    "/permissions/{scope}/{scope_id}/{action}",
    Scope::Admin,
    true,
);

pub const ROUTES: &[Route] = &[
    r("GET", "/permissions", Scope::Read, false),
    PUT_ROUTE,
    r("GET", "/permission-requests/{id}", Scope::Read, false),
    r(
        "POST",
        "/permission-requests/{id}/decision",
        Scope::Admin,
        true,
    ),
    r("GET", "/actions", Scope::Read, false),
    r("GET", "/observer-findings", Scope::Read, false),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

/// `PUT /permissions/{scope}/{scope_id}/{action}`: three path parameters, the `{id}` template does not take them.
pub fn match_put(method: &str, path: &str) -> Option<&'static Route> {
    (method == "PUT"
        && path
            .strip_prefix("/permissions/")
            .is_some_and(|p| p.split('/').count() == 3))
    .then_some(&PUT_ROUTE)
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/permissions") => matrix(ctx),
        ("PUT", _) => put(ctx),
        ("GET", "/permission-requests/{id}") => get_request(ctx, id.ok_or_else(not_found)?),
        ("POST", "/permission-requests/{id}/decision") => decide(ctx, id.ok_or_else(not_found)?),
        ("GET", "/actions") => crate::log_http::actions(ctx),
        ("GET", "/observer-findings") => crate::log_http::findings(ctx),
        _ => Err(not_found()),
    }
}

fn matrix(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let (mut scope, mut id) = (None, None);
    for (k, v) in query_pairs(ctx.query) {
        match k {
            "scope" if ["global", "persona", "session"].contains(&v) => scope = Some(v),
            "scope" => {
                return Err(ApiErr::new(
                    400,
                    "bad_request",
                    "scope: global, persona or session",
                ));
            }
            "scope_id" => id = Some(int_param(v, k)?),
            _ => {}
        }
    }
    let conn = ctx.shared.db.reader().map_err(internal)?;
    Ok(Reply::json(
        200,
        Value::Array(store::permissions_json(&conn, scope, id).map_err(internal)?),
    ))
}

fn put(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let parts: Vec<&str> = ctx
        .path
        .trim_start_matches("/v1/permissions/")
        .split('/')
        .collect();
    let [scope, sid, action] = parts[..] else {
        return Err(not_found());
    };
    if !["global", "persona", "session"].contains(&scope) {
        return Err(invalid("scope", "scope: global, persona or session"));
    }
    let sid = sid
        .parse::<i64>()
        .map_err(|_| invalid("scope_id", "scope_id: integer"))?;
    if (scope == "global") != (sid == 0) {
        return Err(invalid(
            "scope_id",
            "for global scope_id = 0, otherwise the level id",
        ));
    }
    if !store::ACTIONS.contains(&action) {
        return Err(invalid("action", "unknown action"));
    }
    let mode = object(ctx)?
        .get("mode")
        .and_then(Value::as_str)
        .and_then(Mode::parse)
        .ok_or_else(|| invalid("mode", "mode: allow, ask or deny"))?;
    let now = ctx.shared.clock.now_unix();
    let done = ctx.shared.db.write(|w| {
        match store::put_rule(w, now, (scope, sid, action), mode, "gui", false) {
            Ok(()) => {
                let target = format!("{scope}/{sid}/{action}={}", mode.as_str());
                store::log_action(
                    w,
                    now,
                    "user",
                    (None, None),
                    ("config_write", Some(&target)),
                    None,
                    (Some("allow"), Some("ok")),
                )?;
                Ok(Ok(()))
            }
            Err(PutError::Loosen) => Ok(Err(())),
            Err(PutError::Db(e)) => Err(e),
        }
    });
    match done.map_err(internal)? {
        Ok(()) => Ok(Reply::json(
            200,
            json!({ "scope": scope, "scope_id": sid, "action": action, "mode": mode.as_str() }),
        )),
        Err(()) => Err(ApiErr::new(
            403,
            "permission_denied",
            "A lower level cannot loosen a higher-level rule",
        )),
    }
}

fn get_request(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    crate::perm_sweep::sweep_expired(ctx.shared);
    let g = ctx.shared.chat.lock();
    g.requests
        .get(&id)
        .map(|r| Reply::json(200, r.json()))
        .ok_or_else(not_found)
}

fn decide(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    if !matches!(ctx.who.kind, Kind::Gui | Kind::Test) {
        return Err(ApiErr::new(
            403,
            "permission_denied",
            "Only the local GUI decides",
        ));
    }
    let m = object(ctx)?;
    let outcome = match m.get("decision").and_then(Value::as_str) {
        Some("allow") => Outcome::Allow,
        Some("deny") => Outcome::Deny,
        Some("always") => Outcome::Allow,
        _ => return Err(invalid("decision", "decision: allow, deny or always")),
    };
    let always = m.get("decision").and_then(Value::as_str) == Some("always");
    let rule = if always { Some(rule_of(&m)?) } else { None };
    let source = ctx.shared.chat.lock().requests.get(&id).map(|r| r.source);
    if always && source == Some("observer") {
        return Err(invalid(
            "decision",
            "\"Always\" is unavailable for watcher confirmation",
        ));
    }
    let rule = rule.as_ref().map(|(s, i, m)| (s.as_str(), *i, *m));
    match perm_flow::resolve(ctx.shared, id, outcome, "user", rule) {
        Ok(req) => Ok(Reply::json(200, req.json())),
        Err(ResolveErr::NotFound) => Err(not_found()),
        Err(ResolveErr::Decided(status)) if status == "expired" => {
            Err(ApiErr::new(409, "conflict", "Request expired")
                .details(json!({ "reason": "expired" })))
        }
        Err(ResolveErr::Decided(status)) => {
            Err(ApiErr::new(409, "conflict", "Request already decided")
                .details(json!({ "reason": "already_decided", "status": status })))
        }
        Err(ResolveErr::Expired) => {
            Err(ApiErr::new(409, "conflict", "Request expired")
                .details(json!({ "reason": "expired" })))
        }
        Err(ResolveErr::Loosen) => Err(ApiErr::new(
            403,
            "permission_denied",
            "The rule would loosen a higher-level rule",
        )),
        Err(ResolveErr::Db) => Err(internal("decision not written")),
    }
}

fn rule_of(m: &serde_json::Map<String, Value>) -> Result<(String, i64, Mode), ApiErr> {
    let rule = m
        .get("rule")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("rule", "rule is required for always"))?;
    let scope = rule
        .get("scope")
        .and_then(Value::as_str)
        .filter(|s| ["persona", "global"].contains(s))
        .ok_or_else(|| invalid("rule.scope", "scope: persona or global"))?;
    let sid = rule.get("scope_id").and_then(Value::as_i64).unwrap_or(0);
    if (scope == "global") != (sid == 0) {
        return Err(invalid(
            "rule.scope_id",
            "for global scope_id = 0, for persona the persona id",
        ));
    }
    let mode = match rule.get("mode") {
        None | Some(Value::Null) => Mode::Allow,
        Some(v) => v
            .as_str()
            .and_then(Mode::parse)
            .ok_or_else(|| invalid("rule.mode", "mode: allow, ask or deny"))?,
    };
    Ok((scope.to_string(), sid, mode))
}
