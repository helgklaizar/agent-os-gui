//! Personas and accounts (api.md section 5.6): `POST/PATCH/DELETE /personas`, `GET/POST/PATCH /accounts`, connecting.
//! `GET /personas` and `DELETE /accounts/{id}` stay in `tree_http` and `bar_http`.

use crate::accounts_http as accounts;
use crate::chat_http::not_found;
use crate::handlers::Ctx;
use crate::personas_http as personas;
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
    r("POST", "/personas", Scope::Admin),
    r("PATCH", "/personas/{id}", Scope::Admin),
    r("DELETE", "/personas/{id}", Scope::Admin),
    r("GET", "/accounts", Scope::Read),
    r("POST", "/accounts", Scope::Admin),
    r("PATCH", "/accounts/{id}", Scope::Admin),
    r("POST", "/accounts/{id}/connect", Scope::Admin),
    r("DELETE", "/accounts/{id}/connect", Scope::Admin),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let need = || id.ok_or_else(not_found);
    match (route.method, route.pattern) {
        ("POST", "/personas") => personas::create(ctx),
        ("PATCH", "/personas/{id}") => personas::patch(ctx, need()?),
        ("DELETE", "/personas/{id}") => personas::delete(ctx, need()?),
        ("GET", "/accounts") => accounts::list(ctx),
        ("POST", "/accounts") => accounts::create(ctx),
        ("PATCH", "/accounts/{id}") => accounts::patch(ctx, need()?),
        ("POST", "/accounts/{id}/connect") => accounts::connect(ctx, need()?),
        _ => accounts::disconnect(ctx, need()?),
    }
}
