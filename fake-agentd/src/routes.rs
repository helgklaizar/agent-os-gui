//! Route table (the `/v1` prefix is stripped): scope and `x-local-only` per `openapi-draft.yaml`.

use crate::auth::Authed;
use crate::reply::ApiErr;
use crate::store::Scope;

pub struct Route {
    pub method: &'static str,
    pub pattern: &'static str,
    pub scope: Scope,
    pub local_only: bool,
}

pub const ROUTES: &[Route] = &[
    Route {
        method: "GET",
        pattern: "/settings/theme",
        scope: Scope::Read,
        local_only: false,
    },
    Route {
        method: "PATCH",
        pattern: "/settings/theme",
        scope: Scope::Chat,
        local_only: false,
    },
    Route {
        method: "GET",
        pattern: "/tokens",
        scope: Scope::Admin,
        local_only: true,
    },
    Route {
        method: "POST",
        pattern: "/tokens",
        scope: Scope::Admin,
        local_only: true,
    },
    Route {
        method: "DELETE",
        pattern: "/tokens/{id}",
        scope: Scope::Admin,
        local_only: true,
    },
    // Service routes of `fake-agentd`, not part of the contract.
    Route {
        method: "POST",
        pattern: "/_test/echo",
        scope: Scope::Chat,
        local_only: false,
    },
    Route {
        method: "POST",
        pattern: "/_test/advance",
        scope: Scope::Admin,
        local_only: true,
    },
];

impl Route {
    /// Token allowlist key: `METHOD /pattern`.
    pub fn key(&self) -> String {
        format!("{} {}", self.method, self.pattern)
    }
}

/// Matches a path to a pattern by segments: `{id}` is an integer (returned), any other `{name}` is any non-empty segment.
fn matches(pattern: &str, path: &str) -> Option<Option<i64>> {
    let (mut pp, mut hp) = (pattern.split('/'), path.split('/'));
    let mut id = None;
    loop {
        match (pp.next(), hp.next()) {
            (None, None) => return Some(id),
            (Some("{id}"), Some(h)) => id = Some(h.parse::<i64>().ok()?),
            (Some(p), Some(h)) if p.starts_with('{') => {
                if h.is_empty() {
                    return None;
                }
            }
            (Some(p), Some(h)) if p == h => {}
            _ => return None,
        }
    }
}

/// Route and numeric `{id}` from the path (the path is already without `/v1`).
pub fn find(method: &str, path: &str) -> Option<(&'static Route, Option<i64>)> {
    if let Some(r) = crate::perm_http::match_put(method, path) {
        return Some((r, None));
    }
    ROUTES
        .iter()
        .chain(crate::bar_http::ROUTES)
        .chain(crate::tree_http::ROUTES)
        .chain(crate::chat_http::ROUTES)
        .chain(crate::perm_http::ROUTES)
        .chain(crate::events_http::ROUTES)
        .chain(crate::tasks_http::ROUTES)
        .chain(crate::sess_http::ROUTES)
        .chain(crate::tools_http::ROUTES)
        .chain(crate::fail_inject::ROUTES)
        .chain(crate::settings_http::ROUTES)
        .chain(crate::secrets_http::ROUTES)
        .chain(crate::people_http::ROUTES)
        .chain(crate::modules_http::ROUTES)
        .chain(crate::tunnels_http::ROUTES)
        .chain(crate::files_http::ROUTES)
        .chain(crate::search_http::ROUTES)
        .chain(crate::ops_http::ROUTES)
        .find_map(|r| {
            if r.method != method {
                return None;
            }
            matches(r.pattern, path).map(|id| (r, id))
        })
}

/// Order: local route over TCP, then scope, then allowlist (`insufficient_scope` and `forbidden_method` are different codes).
pub fn authorize(route: &Route, who: &Authed, tcp: bool) -> Result<(), ApiErr> {
    if route.local_only && tcp {
        return Err(ApiErr::new(
            403,
            "insufficient_scope",
            "Route is available only over the local socket",
        )
        .details(serde_json::json!({ "reason": "local_only" })));
    }
    if who.scope < route.scope {
        return Err(ApiErr::new(
            403,
            "insufficient_scope",
            "Token lacks the required scope",
        ));
    }
    if who
        .allowlist
        .as_ref()
        .is_some_and(|a| !a.contains(&route.key()))
    {
        return Err(ApiErr::new(
            403,
            "forbidden_method",
            "Method is outside the token allowlist",
        ));
    }
    Ok(())
}
