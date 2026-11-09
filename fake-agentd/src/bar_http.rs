//! Bar routes (`/quotas`, `DELETE /accounts/{id}`, `POST /logout`): table and handlers.

use rusqlite::params;
use serde_json::Value;

use crate::bar;
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;

const fn r(method: &'static str, pattern: &'static str, scope: Scope, local_only: bool) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/quotas", Scope::Read, false),
    r("DELETE", "/accounts/{id}", Scope::Admin, false),
    r("POST", "/logout", Scope::Admin, true),
];

fn internal(e: impl std::fmt::Display) -> ApiErr {
    tracing::error!(error = %e, "fake-agentd storage failure");
    ApiErr::new(500, "internal", "Storage failure")
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/quotas") => quotas(ctx),
        ("DELETE", "/accounts/{id}") => account_delete(ctx, id),
        ("POST", "/logout") => Ok(Reply::no_content()),
        _ => Err(ApiErr::new(404, "not_found", "Route not found")),
    }
}

fn quotas(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let now = ctx.shared.clock.now_unix();
    let items = bar::quota_items(&ctx.shared.db, now).map_err(internal)?;
    Ok(Reply::json(200, Value::Array(items)))
}

/// Soft delete (db.md section 5): `deleted_at`, clearing `quotas`, `quota_samples` and `secret_ref`; the account row stays.
fn account_delete(ctx: &Ctx<'_>, id: Option<i64>) -> Result<Reply, ApiErr> {
    let id = id.ok_or_else(|| ApiErr::new(404, "not_found", "Account not found"))?;
    let now = ctx.shared.clock.now_unix();
    let done = ctx
        .shared
        .db
        .write(|w| {
            let tx = w.transaction()?;
            let n = tx.execute(
                "UPDATE accounts SET deleted_at = ?1, secret_ref = NULL WHERE id = ?2 AND deleted_at IS NULL",
                params![now, id],
            )?;
            if n == 1 {
                tx.execute("DELETE FROM quotas WHERE account_id = ?1", params![id])?;
                tx.execute("DELETE FROM quota_samples WHERE account_id = ?1", params![id])?;
            }
            tx.commit()?;
            Ok(n == 1)
        })
        .map_err(internal)?;
    if !done {
        return Err(ApiErr::new(404, "not_found", "Account not found"));
    }
    bar::publish_tables(ctx.shared, &ctx.shared.db).map_err(internal)?;
    Ok(Reply::no_content())
}
