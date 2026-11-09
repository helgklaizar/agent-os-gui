//! `/v1` route table: method, path template, minimum scope, "local only".
//! Source: `docs/reference/openapi-draft.yaml` (`x-scope`, `x-local-only`); the check against the file is the test
//! `route_table_matches_openapi` (`agentd/api/tests/contract.rs`). `GET /health` is anonymous and not in the table.

use agentd_core::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Route {
    pub method: &'static str,
    /// Template relative to `/v1`, substitutions in curly braces.
    pub template: &'static str,
    pub scope: Scope,
    pub local_only: bool,
}

const fn r(method: &'static str, template: &'static str, scope: Scope, local_only: bool) -> Route {
    Route {
        method,
        template,
        scope,
        local_only,
    }
}

pub(crate) const ROUTES: &[Route] = &[
    r("GET", "/metrics", Scope::Admin, true),
    r("POST", "/logout", Scope::Admin, true),
    r("GET", "/tokens", Scope::Admin, true),
    r("POST", "/tokens", Scope::Admin, true),
    r("DELETE", "/tokens/{id}", Scope::Admin, true),
    r("GET", "/sessions", Scope::Read, false),
    r("POST", "/sessions", Scope::Chat, false),
    r("GET", "/sessions/{id}", Scope::Read, false),
    r("PATCH", "/sessions/{id}", Scope::Chat, false),
    r("DELETE", "/sessions/{id}", Scope::Chat, false),
    r("POST", "/sessions/{id}/threads", Scope::Chat, false),
    r("POST", "/sessions/{id}/read", Scope::Chat, false),
    r("POST", "/sessions/{id}/clear", Scope::Chat, false),
    r("POST", "/sessions/{id}/export", Scope::Chat, false),
    r("GET", "/sessions/{id}/messages", Scope::Read, false),
    r("POST", "/sessions/{id}/messages", Scope::Chat, false),
    r("POST", "/sessions/{id}/stop", Scope::Chat, false),
    r("POST", "/sessions/{id}/tool-results", Scope::Chat, false),
    r("POST", "/sessions/{id}/attachments", Scope::Chat, false),
    r("GET", "/attachments/{id}", Scope::Read, false),
    r("GET", "/sessions/{id}/tasks", Scope::Read, false),
    r("POST", "/sessions/{id}/tasks", Scope::Chat, false),
    r("GET", "/sessions/{id}/bookmarks", Scope::Read, false),
    r("PUT", "/sessions/{id}/bookmarks", Scope::Chat, false),
    r("GET", "/browser/quicklinks", Scope::Read, false),
    r("PUT", "/browser/quicklinks", Scope::Chat, false),
    r("GET", "/folders", Scope::Read, false),
    r("POST", "/folders", Scope::Chat, false),
    r("PATCH", "/folders/{id}", Scope::Chat, false),
    r("DELETE", "/folders/{id}", Scope::Chat, false),
    r("GET", "/events", Scope::Read, false),
    r("GET", "/actions", Scope::Read, false),
    r("POST", "/events/read", Scope::Chat, false),
    r("GET", "/events/subtypes", Scope::Read, false),
    r("POST", "/events/{id}/read", Scope::Chat, false),
    r("GET", "/permissions", Scope::Read, false),
    r(
        "PUT",
        "/permissions/{scope}/{scope_id}/{action}",
        Scope::Admin,
        true,
    ),
    r("GET", "/permission-requests/{id}", Scope::Read, false),
    r(
        "POST",
        "/permission-requests/{id}/decision",
        Scope::Admin,
        true,
    ),
    r("GET", "/search", Scope::Read, false),
    r("GET", "/personas", Scope::Read, false),
    r("POST", "/personas", Scope::Admin, false),
    r("PATCH", "/personas/{id}", Scope::Admin, false),
    r("DELETE", "/personas/{id}", Scope::Admin, false),
    r("GET", "/accounts", Scope::Read, false),
    r("POST", "/accounts", Scope::Admin, false),
    r("PATCH", "/accounts/{id}", Scope::Admin, false),
    r("DELETE", "/accounts/{id}", Scope::Admin, false),
    r("POST", "/accounts/{id}/connect", Scope::Admin, false),
    r("DELETE", "/accounts/{id}/connect", Scope::Admin, false),
    r("GET", "/quotas", Scope::Read, false),
    r("GET", "/modules", Scope::Read, false),
    r("POST", "/modules", Scope::Admin, false),
    r("PATCH", "/modules/{module_id}", Scope::Admin, false),
    r("DELETE", "/modules/{module_id}", Scope::Admin, false),
    r("POST", "/modules/{module_id}/token", Scope::Admin, false),
    r("GET", "/tasks", Scope::Read, false),
    r("PATCH", "/tasks/{id}", Scope::Chat, false),
    r("DELETE", "/tasks/{id}", Scope::Chat, false),
    r("GET", "/tasks/{id}/runs", Scope::Read, false),
    r("POST", "/tasks/{id}/run", Scope::Chat, false),
    r("GET", "/triggers", Scope::Read, false),
    r("POST", "/triggers", Scope::Admin, false),
    r("PATCH", "/triggers/{id}", Scope::Admin, false),
    r("DELETE", "/triggers/{id}", Scope::Admin, false),
    r("GET", "/tunnels", Scope::Admin, false),
    r("POST", "/tunnels", Scope::Admin, false),
    r("PATCH", "/tunnels/{id}", Scope::Admin, false),
    r("DELETE", "/tunnels/{id}", Scope::Admin, false),
    r("GET", "/secrets", Scope::Admin, true),
    r("PUT", "/secrets/{ref}", Scope::Admin, true),
    r("DELETE", "/secrets/{ref}", Scope::Admin, true),
    r("GET", "/files", Scope::Admin, true),
    r("GET", "/files/content", Scope::Admin, true),
    r("PUT", "/files/content", Scope::Admin, true),
    r("GET", "/settings/indicators", Scope::Read, false),
    r("PATCH", "/settings/indicators", Scope::Admin, false),
    r("GET", "/settings/theme", Scope::Read, false),
    r("PATCH", "/settings/theme", Scope::Chat, false),
    r("GET", "/settings/{page}", Scope::Read, false),
    r("PATCH", "/settings/{page}", Scope::Admin, false),
];

/// Test-build routes (A1-11); absent from release builds.
#[cfg(feature = "test-support")]
pub(crate) const TEST_ROUTES: &[Route] = &[
    r("POST", "/_test/advance", Scope::Admin, true),
    r("POST", "/_test/observer", Scope::Admin, true),
    r("POST", "/_test/indicator", Scope::Admin, true),
    r("POST", "/_test/ingest", Scope::Admin, true),
    r("POST", "/_test/channel", Scope::Admin, true),
];
#[cfg(not(feature = "test-support"))]
pub(crate) const TEST_ROUTES: &[Route] = &[];

fn all() -> impl Iterator<Item = &'static Route> + Clone {
    ROUTES.iter().chain(TEST_ROUTES.iter())
}

/// What was found by method and path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    Found(&'static Route),
    /// The path is known but the method is not: allowed methods for `Allow`.
    MethodNotAllowed(Vec<&'static str>),
    NotFound,
}

fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/').split('/').collect()
}

/// How many literal segments matched; `None` means the template does not fit. A literal beats a substitution
/// (`/settings/theme` versus `/settings/{page}`).
fn specificity(template: &str, path: &[&str]) -> Option<usize> {
    let t = segments(template);
    if t.len() != path.len() {
        return None;
    }
    let mut literals = 0;
    for (a, b) in t.iter().zip(path) {
        if a.starts_with('{') {
            if b.is_empty() {
                return None;
            }
        } else if a == b {
            literals += 1;
        } else {
            return None;
        }
    }
    Some(literals)
}

/// `path` is without the `/v1` prefix.
pub(crate) fn lookup(method: &str, path: &str) -> Lookup {
    let segs = segments(path);
    let best = all()
        .filter_map(|r| specificity(r.template, &segs).map(|s| (s, r)))
        .map(|(s, _)| s)
        .max();
    let Some(best) = best else {
        return Lookup::NotFound;
    };
    let same: Vec<&'static Route> = all()
        .filter(|r| specificity(r.template, &segs) == Some(best))
        .collect();
    if let Some(found) = same.iter().find(|r| r.method == method) {
        return Lookup::Found(found);
    }
    // A more general template could match the path by method (`PATCH /settings/theme` exists, but `PUT` exists nowhere).
    let mut allow: Vec<&'static str> = same.iter().map(|r| r.method).collect();
    allow.dedup();
    Lookup::MethodNotAllowed(allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_beats_placeholder() {
        let Lookup::Found(r) = lookup("GET", "/settings/theme") else {
            panic!()
        };
        assert_eq!(r.template, "/settings/theme");
        let Lookup::Found(r) = lookup("GET", "/settings/models") else {
            panic!()
        };
        assert_eq!(r.template, "/settings/{page}");
        let Lookup::Found(r) = lookup("POST", "/events/read") else {
            panic!()
        };
        assert_eq!(r.template, "/events/read");
        let Lookup::Found(r) = lookup("POST", "/events/5/read") else {
            panic!()
        };
        assert_eq!(r.template, "/events/{id}/read");
    }

    #[test]
    fn unknown_and_wrong_method() {
        assert_eq!(lookup("GET", "/nope"), Lookup::NotFound);
        assert_eq!(lookup("GET", "/sessions//messages"), Lookup::NotFound);
        assert_eq!(lookup("GET", "/"), Lookup::NotFound);
        let Lookup::MethodNotAllowed(a) = lookup("PUT", "/sessions") else {
            panic!()
        };
        assert!(a.contains(&"GET") && a.contains(&"POST"));
    }

    /// The table matches `openapi-draft.yaml` in both directions: method, path, `x-scope`, `x-local-only`.
    #[test]
    fn route_table_matches_openapi() {
        let yaml = include_str!("../../../docs/reference/openapi-draft.yaml");
        let mut path = String::new();
        let mut spec: Vec<(String, String, String, bool)> = Vec::new();
        for line in yaml.lines() {
            if line.starts_with("components:") {
                break;
            }
            if let Some(p) = line.strip_prefix("  /").and_then(|p| p.strip_suffix(':')) {
                path = format!("/{p}");
            } else if let Some(m) = line
                .strip_prefix("    ")
                .and_then(|m| m.strip_suffix(':'))
                .filter(|m| ["get", "post", "put", "patch", "delete"].contains(m))
            {
                spec.push((m.to_ascii_uppercase(), path.clone(), String::new(), false));
            } else if let Some(last) = spec.last_mut() {
                if let Some(sc) = line.strip_prefix("      x-scope: ") {
                    last.2 = sc.trim().to_string();
                }
                if line == "      x-local-only: true" {
                    last.3 = true;
                }
            }
        }
        // `GET /health` is anonymous (no `x-scope`) and not in the table.
        spec.retain(|r| !r.2.is_empty());
        let mut table: Vec<(String, String, String, bool)> = ROUTES
            .iter()
            .map(|r| {
                (
                    r.method.to_string(),
                    r.template.to_string(),
                    r.scope.as_str().to_string(),
                    r.local_only,
                )
            })
            .collect();
        spec.sort();
        table.sort();
        assert_eq!(table.len(), spec.len(), "route count");
        assert_eq!(table, spec);
    }
}
