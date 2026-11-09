//! Session actions (api.md section 5.1): clearing history and export. The core-level `delete` permission is not emulated here:
//! the GUI asks for the "what will be lost" confirmation before the call.

use rusqlite::params;
use serde_json::json;

use crate::chat_http::{internal, not_found};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;
use crate::tasks_http::session_exists;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("POST", "/sessions/{id}/clear", Scope::Chat),
    r("POST", "/sessions/{id}/export", Scope::Chat),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let id = id.ok_or_else(not_found)?;
    session_exists(ctx, id)?;
    match route.pattern {
        "/sessions/{id}/clear" => {
            ctx.shared
                .db
                .write(|w| {
                    w.execute("DELETE FROM messages WHERE session_id = ?1", params![id])
                        .map(|_| ())
                })
                .map_err(internal)?;
            Ok(Reply::no_content())
        }
        "/sessions/{id}/export" => {
            let conn = ctx.shared.db.reader().map_err(internal)?;
            // export size: message text plus a header; the mock does not write a file
            let bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(length(CAST(body_md AS BLOB))), 0) FROM messages WHERE session_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(internal)?;
            let name = format!("# Session {id}\n");
            Ok(Reply::json(
                200,
                json!({
                    "path": format!("~/.agentos/export/session-{id}.md"),
                    "size": bytes + i64::try_from(name.len()).unwrap_or(0),
                }),
            ))
        }
        _ => Err(not_found()),
    }
}
