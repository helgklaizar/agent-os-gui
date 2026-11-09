//! Failure injection for tests: `POST /v1/_test/fail` and `DELETE /v1/_test/fail`. Service routes, not part of the
//! contract. The next N requests by key `METHOD /v1/path` (path without query) get the given error, data does not change.

use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::chat_http::{invalid, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;

pub const ROUTES: &[Route] = &[
    Route {
        method: "POST",
        pattern: "/_test/fail",
        scope: Scope::Admin,
        local_only: true,
    },
    Route {
        method: "DELETE",
        pattern: "/_test/fail",
        scope: Scope::Admin,
        local_only: true,
    },
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

/// Allowed failure codes.
pub const STATUSES: [u16; 4] = [500, 503, 422, 412];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Failure queue: key, code, how many times are left.
#[derive(Default)]
pub struct Fails(Mutex<Vec<(String, u16, u32)>>);

impl Fails {
    pub fn add(&self, route: &str, status: u16, times: u32) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push((
            route.to_string(),
            status,
            times,
        ));
    }

    pub fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Failure code for a request if it is queued (the counter decreases).
    pub fn take(&self, key: &str) -> Option<u16> {
        let mut q = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let i = q.iter().position(|(k, _, n)| k == key && *n > 0)?;
        q[i].2 -= 1;
        let status = q[i].1;
        if q[i].2 == 0 {
            q.remove(i);
        }
        Some(status)
    }
}

/// Error with the usual body (api.md section 3) by failure code.
pub fn error_for(status: u16) -> ApiErr {
    match status {
        503 => ApiErr::new(503, "core_down", "Failure (test injection)").retry_after(1),
        422 => ApiErr::new(422, "validation_failed", "Failure (test injection)"),
        412 => ApiErr::new(412, "precondition_failed", "Failure (test injection)"),
        _ => ApiErr::new(500, "internal", "Failure (test injection)"),
    }
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    if route.method == "DELETE" {
        ctx.shared.fail.clear();
        return Ok(Reply::no_content());
    }
    let m = object(ctx)?;
    let key = m.get("route").and_then(Value::as_str).unwrap_or("");
    let ok_key = key.split_once(' ').is_some_and(|(method, path)| {
        METHODS.contains(&method) && path.starts_with("/v1/") && !path.contains('?')
    });
    if !ok_key {
        return Err(ApiErr::new(
            400,
            "bad_request",
            "route: \"METHOD /v1/path\" without query",
        ));
    }
    let status = m
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .filter(|s| STATUSES.contains(s))
        .ok_or_else(|| ApiErr::new(400, "bad_request", "status: 500, 503, 422 or 412"))?;
    let times = match m.get("times") {
        None => 1,
        Some(t) => t
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=1000).contains(n))
            .ok_or_else(|| invalid("times", "integer from 1 to 1000"))?,
    };
    ctx.shared.fail.add(key, status, times);
    Ok(Reply::no_content())
}

impl crate::FakeAgentd {
    /// The next `times` requests by key `METHOD /v1/path` get the error `status` (500, 503, 422 or 412), data does not change.
    pub fn fail_next(&self, route: &str, status: u16, times: u32) {
        self.shared.fail.add(route, status, times);
    }
}
