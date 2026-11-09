//! Task routes (api.md section 5.8): `/sessions/{id}/tasks`, `/tasks`, `/tasks/{id}`, `/tasks/{id}/runs`, `/tasks/{id}/run`.

use serde_json::Value;

use crate::chat_http::{internal, invalid, not_found, object, query_pairs};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;
use crate::tasks_ops::{self as ops, Bad, Filter};
use crate::tree;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/sessions/{id}/tasks", Scope::Read),
    r("POST", "/sessions/{id}/tasks", Scope::Chat),
    r("GET", "/tasks", Scope::Read),
    r("PATCH", "/tasks/{id}", Scope::Chat),
    r("DELETE", "/tasks/{id}", Scope::Chat),
    r("GET", "/tasks/{id}/runs", Scope::Read),
    r("POST", "/tasks/{id}/run", Scope::Chat),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

fn from_bad(b: Bad) -> ApiErr {
    invalid(b.field, b.why)
}

pub fn session_exists(ctx: &Ctx<'_>, id: i64) -> Result<(), ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let t = tree::load(&conn).map_err(internal)?;
    t.session(id).map(|_| ()).ok_or_else(not_found)
}

/// `kind`, `state`, `session_id` filter of `GET /tasks`.
type TaskQuery = (Option<String>, Option<String>, Option<i64>);

fn filter(query: Option<&str>) -> Result<TaskQuery, ApiErr> {
    let (mut kind, mut state, mut session) = (None, None, None);
    for (k, v) in query_pairs(query) {
        match k {
            "kind" if ["sched", "deleg"].contains(&v) => kind = Some(v.to_string()),
            "state" if ["active", "paused", "done"].contains(&v) => state = Some(v.to_string()),
            "session_id" => match v.parse() {
                Ok(n) => session = Some(n),
                Err(_) => return Err(invalid("session_id", "integer")),
            },
            "kind" => return Err(invalid("kind", "sched or deleg")),
            "state" => return Err(invalid("state", "active, paused or done")),
            _ => {}
        }
    }
    Ok((kind, state, session))
}

fn runs_params(query: Option<&str>) -> Result<(Option<i64>, i64), ApiErr> {
    let (mut before, mut limit) = (None, ops::RUNS_DEFAULT_LIMIT);
    for (k, v) in query_pairs(query) {
        match k {
            "before" => match v.parse::<i64>() {
                Ok(n) if n > 0 => before = Some(n),
                _ => return Err(invalid("before", "positive integer")),
            },
            "limit" => match v.parse::<i64>() {
                Ok(n) if (1..=ops::RUNS_MAX_LIMIT).contains(&n) => limit = n,
                _ => return Err(invalid("limit", "integer from 1 to 200")),
            },
            _ => {}
        }
    }
    Ok((before, limit))
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    // `/tasks` has no number; the others always have one (parsed by `routes::find`), 0 means "not found"
    let id = id.unwrap_or(0);
    match (route.method, route.pattern) {
        ("GET", "/sessions/{id}/tasks") => {
            session_exists(ctx, id)?;
            let conn = ctx.shared.db.reader().map_err(internal)?;
            let f = Filter {
                session_id: Some(id),
                ..Filter::default()
            };
            Ok(Reply::json(
                200,
                Value::Array(ops::list(&conn, &f).map_err(internal)?),
            ))
        }
        ("POST", "/sessions/{id}/tasks") => {
            session_exists(ctx, id)?;
            let fields = ops::parse(&object(ctx)?, true).map_err(from_bad)?;
            let now = ctx.shared.clock.now_unix();
            let made = ctx
                .shared
                .db
                .write(|w| ops::insert(w, id, &fields, now))
                .map_err(|e| {
                    if e.to_string().contains("FOREIGN KEY") {
                        invalid("persona_id", "no such persona")
                    } else {
                        internal(e)
                    }
                })?;
            Ok(Reply::json(201, made))
        }
        ("GET", "/tasks") => {
            let (kind, state, session) = filter(ctx.query)?;
            let conn = ctx.shared.db.reader().map_err(internal)?;
            let f = Filter {
                kind: kind.as_deref(),
                state: state.as_deref(),
                session_id: session,
            };
            Ok(Reply::json(
                200,
                Value::Array(ops::list(&conn, &f).map_err(internal)?),
            ))
        }
        ("PATCH", "/tasks/{id}") => {
            let fields = ops::parse(&object(ctx)?, false).map_err(from_bad)?;
            if fields.kind.is_some() {
                return Err(invalid("kind", "task kind cannot change"));
            }
            let task = ctx
                .shared
                .db
                .write(|w| ops::update(w, id, &fields))
                .map_err(|e| {
                    if e.to_string().contains("FOREIGN KEY") {
                        invalid("persona_id", "no such persona")
                    } else {
                        internal(e)
                    }
                })?;
            task.map(|t| Reply::json(200, t)).ok_or_else(not_found)
        }
        ("DELETE", "/tasks/{id}") => {
            let gone = ctx
                .shared
                .db
                .write(|w| ops::delete(w, id))
                .map_err(internal)?;
            if gone {
                Ok(Reply::no_content())
            } else {
                Err(not_found())
            }
        }
        ("GET", "/tasks/{id}/runs") => {
            let (before, limit) = runs_params(ctx.query)?;
            let conn = ctx.shared.db.reader().map_err(internal)?;
            if ops::get(&conn, id).map_err(internal)?.is_none() {
                return Err(not_found());
            }
            Ok(Reply::json(
                200,
                Value::Array(ops::runs(&conn, id, before, limit).map_err(internal)?),
            ))
        }
        ("POST", "/tasks/{id}/run") => ops::run_now(ctx.shared, id)
            .map_err(internal)?
            .map(|run| Reply::json(202, run))
            .ok_or_else(not_found),
        _ => Err(not_found()),
    }
}
