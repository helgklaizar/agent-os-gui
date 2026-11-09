//! Tunnels (`docs/reference/db.md` 3.9, `docs/reference/api.md` 5.7): types, secret-free field validation, store port. Tunnel secrets
//! (keys, tokens) are stored only through `PUT /secrets/{ref}`; they must not appear in `addr` and `descr`.

use serde_json::{Map, Value, json};

use crate::{FieldError, PortFuture};

pub const TUNNEL_TYPES: [&str; 5] = ["wireguard", "tailscale", "ssh", "cloudflare", "ngrok"];
const MAX_ADDR: usize = 200;
const MAX_DESCR: usize = 200;
/// Types the core manages (process launch); the rest are "external".
const MANAGED: [&str; 2] = ["wireguard", "ssh"];
const SECRET_WORDS: [&str; 9] = [
    "password",
    "passwd",
    "token",
    "secret",
    "apikey",
    "api_key",
    "privatekey",
    "private_key",
    "key=",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tunnel {
    pub id: i64,
    pub kind: String,
    pub addr: String,
    pub descr: String,
    pub enabled: bool,
}

impl Tunnel {
    /// `up|down|error` for managed types, `external` — "managed outside the core" (a plan assumption).
    #[must_use]
    pub fn status(&self) -> &'static str {
        if MANAGED.contains(&self.kind.as_str()) {
            "down"
        } else {
            "external"
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        self.to_json_with(self.status())
    }

    #[must_use]
    pub fn to_json_with(&self, status: &str) -> Value {
        json!({"id": self.id, "type": self.kind, "addr": self.addr, "descr": self.descr,
               "enabled": self.enabled, "status": status})
    }
}

/// Tunnel process statuses (`up|down|error|external`): written by the process manager, read by the API.
#[derive(Clone, Default)]
pub struct TunnelStatuses(
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<i64, &'static str>>>,
);

impl TunnelStatuses {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<i64, &'static str>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn set(&self, id: i64, status: &'static str) {
        self.lock().insert(id, status);
    }

    pub fn remove(&self, id: i64) {
        self.lock().remove(&id);
    }

    /// Tunnel status; the manager has not seen it yet — a value computed from the type.
    #[must_use]
    pub fn of(&self, t: &Tunnel) -> &'static str {
        self.lock()
            .get(&t.id)
            .copied()
            .unwrap_or_else(|| t.status())
    }
}

#[derive(Debug, PartialEq)]
pub enum TunnelError {
    NotFound,
    Invalid(Vec<FieldError>),
    Unavailable,
}

pub trait TunnelStore: Send + Sync + 'static {
    fn create(&self, t: Tunnel) -> PortFuture<'_, Result<Tunnel, TunnelError>>;
    fn get(&self, id: i64) -> PortFuture<'_, Result<Tunnel, TunnelError>>;
    fn list(&self) -> PortFuture<'_, Result<Vec<Tunnel>, TunnelError>>;
    fn update(&self, t: Tunnel) -> PortFuture<'_, Result<Tunnel, TunnelError>>;
    fn delete(&self, id: i64) -> PortFuture<'_, Result<(), TunnelError>>;
}

/// Whether a value looks like a secret: keywords, credentials in the address, a long continuous key string.
#[must_use]
pub fn looks_secret(s: &str) -> bool {
    let low = s.to_lowercase();
    if SECRET_WORDS.iter().any(|w| low.contains(w)) {
        return true;
    }
    if let Some((_, rest)) = s.split_once("://") {
        let authority = rest.split(['/', '?']).next().unwrap_or_default();
        if authority.contains('@') && authority.split('@').next().is_some_and(|u| u.contains(':')) {
            return true;
        }
    } else if s.split('@').next().is_some_and(|u| u.contains(':')) && s.contains('@') {
        return true;
    }
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='))
        .any(|run| run.len() >= 40)
}

fn text(
    v: Option<&Value>,
    field: &str,
    max: usize,
    required: bool,
    errs: &mut Vec<FieldError>,
) -> Option<String> {
    let Some(v) = v else {
        if required {
            errs.push(FieldError::new(field, "required field"));
        }
        return None;
    };
    match v.as_str().map(str::trim) {
        Some(s) if (!s.is_empty() || !required) && s.chars().count() <= max => {
            if field == "addr" && s.starts_with('-') {
                errs.push(FieldError::new(
                    field,
                    "an address cannot start with \"-\" (it is a command parameter)",
                ));
                None
            } else if looks_secret(s) {
                errs.push(FieldError::new(
                    field,
                    "secrets are stored through PUT /secrets/{ref}, not here",
                ));
                None
            } else {
                Some(s.to_string())
            }
        }
        _ => {
            errs.push(FieldError::new(
                field,
                &format!("a non-empty string up to {max} characters"),
            ));
            None
        }
    }
}

fn unknown(obj: &Map<String, Value>, errs: &mut Vec<FieldError>) {
    for k in obj
        .keys()
        .filter(|k| !["type", "addr", "descr", "enabled"].contains(&k.as_str()))
    {
        errs.push(FieldError::new(k, "unknown field"));
    }
}

fn kind(v: Option<&Value>, errs: &mut Vec<FieldError>) -> Option<String> {
    match v.and_then(Value::as_str) {
        Some(k) if TUNNEL_TYPES.contains(&k) => Some(k.to_string()),
        _ => {
            errs.push(FieldError::new(
                "type",
                "wireguard, tailscale, ssh, cloudflare or ngrok",
            ));
            None
        }
    }
}

fn enabled(v: Option<&Value>, errs: &mut Vec<FieldError>) -> Option<bool> {
    match v {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            errs.push(FieldError::new("enabled", "true or false"));
            None
        }
    }
}

/// `POST /tunnels` body; the store assigns `id`.
pub fn parse_create(body: &Value) -> Result<Tunnel, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![FieldError::new("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(obj, &mut errs);
    let k = kind(obj.get("type"), &mut errs);
    let addr = text(obj.get("addr"), "addr", MAX_ADDR, true, &mut errs);
    let descr = text(obj.get("descr"), "descr", MAX_DESCR, false, &mut errs);
    let en = enabled(obj.get("enabled"), &mut errs);
    match (k, addr, errs.is_empty()) {
        (Some(kind), Some(addr), true) => Ok(Tunnel {
            id: 0,
            kind,
            addr,
            descr: descr.unwrap_or_default(),
            enabled: en.unwrap_or(false),
        }),
        _ => Err(errs),
    }
}

/// `PATCH /tunnels/{id}` body: any subset of fields on top of `cur`.
pub fn parse_patch(cur: &Tunnel, body: &Value) -> Result<Tunnel, Vec<FieldError>> {
    let Some(obj) = body.as_object() else {
        return Err(vec![FieldError::new("body", "an object is expected")]);
    };
    let mut errs = Vec::new();
    unknown(obj, &mut errs);
    let mut t = cur.clone();
    if obj.contains_key("type")
        && let Some(k) = kind(obj.get("type"), &mut errs)
    {
        t.kind = k;
    }
    if obj.contains_key("addr")
        && let Some(a) = text(obj.get("addr"), "addr", MAX_ADDR, true, &mut errs)
    {
        t.addr = a;
    }
    if obj.contains_key("descr")
        && let Some(d) = text(obj.get("descr"), "descr", MAX_DESCR, false, &mut errs)
    {
        t.descr = d;
    }
    if let Some(e) = enabled(obj.get("enabled"), &mut errs) {
        t.enabled = e;
    }
    if errs.is_empty() { Ok(t) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_detection() {
        for bad in [
            "user:pw@host",
            "https://u:p@h/x",
            "x?token=1",
            "password: a",
            &"A".repeat(44),
        ] {
            assert!(looks_secret(bad), "{bad}");
        }
        for ok in [
            "vpn.example.test:51820",
            "https://tunnel.example.test/path",
            "home",
            "10.0.0.1",
        ] {
            assert!(!looks_secret(ok), "{ok}");
        }
    }
}
