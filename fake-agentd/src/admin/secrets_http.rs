//! Secrets (api.md section 5.7): write-only, the value is never read back. Values live outside the tables, in the
//! `secrets.env` file of the data directory (mode 0600), and never reach responses, frames, errors or the log.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use crate::chat_http::{internal, invalid, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::settings_conn::valid_secret_ref;
use crate::store::Scope;

const fn r(method: &'static str, pattern: &'static str) -> Route {
    Route {
        method,
        pattern,
        scope: Scope::Admin,
        local_only: true,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/secrets"),
    r("PUT", "/secrets/{ref}"),
    r("DELETE", "/secrets/{ref}"),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

/// Longest secret value, in bytes.
const MAX_VALUE: usize = 8192;

/// Value store: lines `ref \t updated_at \t hex(value)`.
pub struct SecretStore {
    path: PathBuf,
    lock: Mutex<()>,
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

impl SecretStore {
    pub fn new(data_dir: &std::path::Path) -> SecretStore {
        SecretStore {
            path: data_dir.join("secrets.env"),
            lock: Mutex::new(()),
        }
    }

    fn rows(&self) -> Vec<(String, i64, String)> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                Some((
                    p.next()?.to_string(),
                    p.next()?.parse().ok()?,
                    p.next()?.to_string(),
                ))
            })
            .collect()
    }

    fn save(&self, rows: &[(String, i64, String)]) -> std::io::Result<()> {
        let body: String = rows
            .iter()
            .map(|(r, t, v)| format!("{r}\t{t}\t{v}\n"))
            .collect();
        std::fs::write(&self.path, body)?;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
    }

    pub fn set(&self, reference: &str, value: &str, now: i64) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut rows = self.rows();
        rows.retain(|(r, _, _)| r != reference);
        rows.push((reference.to_string(), now, hex(value)));
        rows.sort();
        self.save(&rows)
    }

    pub fn remove(&self, reference: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut rows = self.rows();
        rows.retain(|(r, _, _)| r != reference);
        self.save(&rows)
    }

    /// Which secrets are set: `(ref, updated_at)`.
    pub fn list(&self) -> Vec<(String, i64)> {
        let _g = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.rows().into_iter().map(|(r, t, _)| (r, t)).collect()
    }

    /// Secret value - only for test checks, not in the API.
    pub fn value(&self, reference: &str) -> Option<String> {
        let _g = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.rows()
            .into_iter()
            .find(|(r, _, _)| r == reference)
            .and_then(|(_, _, v)| unhex(&v))
    }
}

fn ref_of<'a>(ctx: &'a Ctx<'_>) -> &'a str {
    ctx.path.strip_prefix("/v1/secrets/").unwrap_or("")
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let store = &ctx.shared.secrets;
    match (route.method, route.pattern) {
        ("GET", "/secrets") => {
            let items: Vec<Value> = store
                .list()
                .into_iter()
                .map(|(r, t)| json!({ "ref": r, "set": true, "updated_at": t }))
                .collect();
            Ok(Reply::json(200, Value::Array(items)))
        }
        ("PUT", _) => {
            let reference = ref_of(ctx);
            if !valid_secret_ref(reference) {
                return Err(invalid(
                    "ref",
                    "reference like provider.<name> or account.<id>",
                ));
            }
            let body = object(ctx)?;
            let value = body
                .get("value")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty() && v.len() <= MAX_VALUE)
                .ok_or_else(|| invalid("value", "string of 1 to 8192 bytes"))?;
            let now = ctx.shared.clock.now_unix();
            store.set(reference, value, now).map_err(internal)?;
            Ok(Reply::json(
                200,
                json!({ "ref": reference, "set": true, "updated_at": now }),
            ))
        }
        _ => {
            let reference = ref_of(ctx);
            if !valid_secret_ref(reference) {
                return Err(invalid(
                    "ref",
                    "reference like provider.<name> or account.<id>",
                ));
            }
            store.remove(reference).map_err(internal)?;
            Ok(Reply::no_content())
        }
    }
}

impl crate::FakeAgentd {
    /// The stored secret value: only for test checks, it cannot be read through the API.
    pub fn secret_value(&self, reference: &str) -> Option<String> {
        self.shared.secrets.value(reference)
    }
}
