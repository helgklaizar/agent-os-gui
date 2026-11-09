//! Modules: a manifest with a strict schema, URL SSRF protection, store port.
//! The panel sandbox runs in the GUI; the core is responsible for the manifest, permissions, token, log and observer check.

use std::net::IpAddr;

use serde_json::{Map, Value, json};

use crate::{FieldError, PortFuture};

pub const PLACEMENTS: [&str; 3] = ["panel", "widget", "button"];
const MANIFEST_FIELDS: [&str; 9] = [
    "id",
    "name",
    "version",
    "entry",
    "placement",
    "permissions",
    "network",
    "fs",
    "questions",
];
/// API scopes a module cannot get (only `admin`).
const FORBIDDEN_PREFIXES: [&str; 11] = [
    "/tokens",
    "/secrets",
    "/files",
    "/permissions",
    "/permission-requests",
    "/settings",
    "/accounts",
    "/modules",
    "/tunnels",
    "/triggers",
    "/logout",
];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
const MAX_LIST: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub manifest: Value,
    pub installed: bool,
    pub enabled: bool,
    pub placement: String,
    pub config: Value,
}

impl Module {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "manifest": self.manifest, "installed": self.installed,
               "enabled": self.enabled, "placement": self.placement, "config": self.config})
    }

    /// Catalog entry: not installed, not enabled.
    #[must_use]
    pub fn from_manifest(manifest: Value, installed: bool) -> Module {
        Module {
            id: manifest["id"].as_str().unwrap_or_default().to_string(),
            name: manifest["name"].as_str().unwrap_or_default().to_string(),
            placement: manifest["placement"]
                .as_str()
                .unwrap_or("panel")
                .to_string(),
            manifest,
            installed,
            enabled: installed,
            config: json!({}),
        }
    }

    /// Bridge methods from the manifest — the module token list (`allowlist_json`).
    #[must_use]
    pub fn permissions(&self) -> Vec<String> {
        self.manifest["permissions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect()
    }
}

#[derive(Debug, PartialEq)]
pub enum ModuleError {
    NotFound,
    /// A module with this `id` is already installed.
    Exists,
    Invalid(Vec<FieldError>),
    Unavailable,
}

pub trait ModuleStore: Send + Sync + 'static {
    fn list(&self) -> PortFuture<'_, Result<Vec<Module>, ModuleError>>;
    fn get(&self, id: String) -> PortFuture<'_, Result<Module, ModuleError>>;
    fn insert(&self, m: Module) -> PortFuture<'_, Result<Module, ModuleError>>;
    fn update(&self, m: Module) -> PortFuture<'_, Result<Module, ModuleError>>;
    fn delete(&self, id: String) -> PortFuture<'_, Result<(), ModuleError>>;
    /// Entries of the local catalog (`~/.agentos/modules/catalog.json`), read-only; broken ones are skipped.
    fn catalog(&self) -> PortFuture<'_, Vec<Value>>;
}

fn err(field: impl Into<String>, msg: &str) -> FieldError {
    FieldError::new(&field.into(), msg)
}

fn valid_id(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_domain(s: &str) -> bool {
    s.contains('.')
        && s.len() <= 253
        && !s.starts_with(['.', '-'])
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

fn valid_permission(p: &str) -> bool {
    let Some((m, path)) = p.split_once(' ') else {
        return false;
    };
    METHODS.contains(&m)
        && path.starts_with('/')
        && path.len() <= 120
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_*-".contains(c))
        && !FORBIDDEN_PREFIXES.iter().any(|f| {
            path == *f || path.starts_with(&format!("{f}/")) || path.starts_with(&format!("{f}*"))
        })
}

fn text(obj: &Map<String, Value>, k: &str, max: usize, errs: &mut Vec<FieldError>) {
    if !obj
        .get(k)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
    {
        errs.push(err(k, &format!("a string of 1 to {max} characters")));
    }
}

fn list_of<'a>(
    obj: &'a Map<String, Value>,
    k: &str,
    errs: &mut Vec<FieldError>,
) -> Vec<(usize, &'a str)> {
    match obj.get(k) {
        None => Vec::new(),
        Some(Value::Array(a)) if a.len() <= MAX_LIST => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.as_str().unwrap_or("")))
            .collect(),
        Some(_) => {
            errs.push(err(k, "an array of strings, at most 32"));
            Vec::new()
        }
    }
}

/// Manifest check: required `id`, `name`, `version`, `entry`, `placement`; an unknown field is a refusal.
pub fn validate_manifest(m: &Value) -> Vec<FieldError> {
    let Some(obj) = m.as_object() else {
        return vec![err("manifest", "an object is expected")];
    };
    let mut errs = Vec::new();
    for k in obj
        .keys()
        .filter(|k| !MANIFEST_FIELDS.contains(&k.as_str()))
    {
        errs.push(err(k.clone(), "unknown field"));
    }
    if !obj.get("id").and_then(Value::as_str).is_some_and(valid_id) {
        errs.push(err("id", "1 to 40 characters of a-z, 0-9 and \"-\""));
    }
    text(obj, "name", 80, &mut errs);
    text(obj, "version", 32, &mut errs);
    match obj.get("entry").and_then(Value::as_str) {
        Some(e)
            if !e.is_empty()
                && e.len() <= 120
                && !e.starts_with('/')
                && !e.contains(['\\', ':', '\0'])
                && !e.split('/').any(|p| p.is_empty() || p == "." || p == "..") => {}
        _ => errs.push(err(
            "entry",
            "a relative path without \"..\" inside the package",
        )),
    }
    if !obj
        .get("placement")
        .and_then(Value::as_str)
        .is_some_and(|p| PLACEMENTS.contains(&p))
    {
        errs.push(err("placement", "panel, widget or button"));
    }
    for (i, p) in list_of(obj, "permissions", &mut errs) {
        if !valid_permission(p) {
            errs.push(err(
                format!("permissions[{i}]"),
                "\"METHOD /path\" outside admin scopes",
            ));
        }
    }
    for (i, d) in list_of(obj, "network", &mut errs) {
        if !valid_domain(d) {
            errs.push(err(
                format!("network[{i}]"),
                "a domain without scheme and wildcards",
            ));
        }
    }
    for (i, f) in list_of(obj, "fs", &mut errs) {
        if !(f.starts_with("~/") && f.len() > 2 && !f.split('/').any(|p| p == ".." || p == ".")) {
            errs.push(err(format!("fs[{i}]"), "an explicit folder like ~/folder"));
        }
    }
    if obj.get("questions").is_some_and(|q| !q.is_array()) {
        errs.push(err("questions", "an array"));
    }
    errs
}

/// Install URL: `http`/`https` only, no credentials, no local or internal addresses (SSRF).
/// The host name is additionally checked at download time (DNS substitution).
pub fn check_install_url(url: &str) -> Result<(), FieldError> {
    check_install_url_allowing(url, None)
}

/// Downloaded manifest limit, bytes.
pub const MAX_DOWNLOAD_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// Network, DNS, timeout, a forbidden address after name resolution.
    Failed,
    /// The server did not answer `200`.
    Status(u16),
    TooLarge,
}

/// Downloading a manifest by URL (implemented by `agentd-providers`): timeout, size limit, address check after
/// name resolution, no redirect following.
pub trait ModuleFetcher: Send + Sync + 'static {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        allowed: Option<&'a str>,
    ) -> PortFuture<'a, Result<Vec<u8>, FetchError>>;
}

/// Same as `check_install_url`, but the `host:port` address named in `allowed` is let through (test build only).
pub fn check_install_url_allowing(url: &str, allowed: Option<&str>) -> Result<(), FieldError> {
    let bad = |m: &str| Err(err("url", m));
    let Some((scheme, rest)) = url.split_once("://") else {
        return bad("an http or https address is required");
    };
    if !matches!(scheme, "http" | "https") {
        return bad("only http and https are allowed");
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return bad("an address without credentials");
    }
    if allowed == Some(authority) {
        return Ok(());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, p)| {
            if p.chars().all(|c| c.is_ascii_digit()) {
                h
            } else {
                authority
            }
        })
    };
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_internal(ip) {
            bad("local and internal addresses are forbidden")
        } else {
            Ok(())
        };
    }
    let low = host.to_lowercase();
    let local = low == "localhost"
        || !low.contains('.')
        || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
            .iter()
            .any(|s| low.ends_with(s));
    if local {
        return bad("local and internal addresses are forbidden");
    }
    Ok(())
}

#[must_use]
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            a.is_loopback()
                || a.is_private()
                || a.is_link_local()
                || a.is_unspecified()
                || a.is_broadcast()
                || a.octets()[0] == 0
                || (a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]))
        }
        IpAddr::V6(a) => {
            a.is_loopback()
                || a.is_unspecified()
                || (a.segments()[0] & 0xfe00) == 0xfc00
                || (a.segments()[0] & 0xffc0) == 0xfe80
                || a.to_ipv4_mapped()
                    .is_some_and(|v4| is_internal(IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_manifest() -> Value {
        json!({"id": "w", "name": "W", "version": "1", "entry": "a.html", "placement": "panel"})
    }

    #[test]
    fn manifest_minimal_is_valid_and_strict() {
        assert!(validate_manifest(&ok_manifest()).is_empty());
        let mut m = ok_manifest();
        m["x"] = json!(1);
        assert_eq!(validate_manifest(&m)[0].field, "x");
    }

    #[test]
    fn urls_to_internal_hosts_are_rejected() {
        for bad in [
            "http://127.0.0.1/x",
            "http://[::1]/x",
            "http://[::ffff:10.0.0.1]/x",
            "http://100.64.0.1/x",
            "http://0.0.0.0/",
            "http://printer/",
            "http://a.local/",
            "ftp://x.test/",
            "http://u@x.test/",
        ] {
            assert!(check_install_url(bad).is_err(), "{bad}");
        }
        assert!(check_install_url("https://modules.example.test/m.json").is_ok());
        assert!(check_install_url("http://93.184.216.34:8080/m.json").is_ok());
    }
}
