//! "Files" editor (api.md section 5.7): only `AGENT.md` and `memory/**/*.md` from `agentos/` of the data directory.
//! Any other path (`keys.env`, `permissions.json`, `..`, absolute, symbolic link) gives `403 permission_denied`.
//! The file `rev` is the first 16 characters of the content SHA-256; no file - `"0"`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::chat_http::{internal, not_found, query_pairs};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::settings_http::stale;
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
    r("GET", "/files"),
    r("GET", "/files/content"),
    r("PUT", "/files/content"),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

const MAX_FILE: usize = 256 * 1024;
/// Decoy: lies on disk next to the editable files and must never appear in API responses.
pub const DECOY_MARKER: &str = "FIXTURE-SECRET-DO-NOT-LEAK";

/// Seeds `agentos/`: editable files and forbidden decoys (existing files are left alone).
pub fn seed(data_dir: &Path) -> std::io::Result<()> {
    let root = data_dir.join("agentos");
    std::fs::create_dir_all(root.join("memory"))?;
    let files: [(&str, String); 5] = [
        ("AGENT.md", "# Agent\n\nAnswer briefly.\n".into()),
        ("memory/notes.md", "# Notes\n\n- first note\n".into()),
        ("memory/ideas.md", "# Ideas\n\n- idea\n".into()),
        ("keys.env", format!("PROVIDER_KEY={DECOY_MARKER}\n")),
        ("permissions.json", "{\"send\":\"ask\"}\n".into()),
    ];
    for (name, body) in files {
        let p = root.join(name);
        if !p.exists() {
            std::fs::write(&p, body)?;
        }
    }
    Ok(())
}

fn deny() -> ApiErr {
    ApiErr::new(
        403,
        "permission_denied",
        "The editor opens only AGENT.md and memory/",
    )
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Valid relative path: `AGENT.md` or `memory/<...>.md` without empty parts, `.` and `..`.
fn allowed(path: &str) -> bool {
    if path == "AGENT.md" {
        return true;
    }
    let Some(rest) = path.strip_prefix("memory/") else {
        return false;
    };
    !path.contains('\0')
        && rest.ends_with(".md")
        && rest.len() > 3
        && rest
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

/// No path component on disk is a symbolic link.
fn no_symlinks(root: &Path, rel: &str) -> bool {
    let mut cur = root.to_path_buf();
    for part in rel.split('/') {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return false,
            Ok(_) | Err(_) => {}
        }
    }
    true
}

fn rev_of(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn info(rel: &str, data: &[u8]) -> Value {
    json!({ "path": rel, "kind": if rel == "AGENT.md" { "prompt" } else { "memory" },
            "size": data.len(), "rev": rev_of(data) })
}

fn root(ctx: &Ctx<'_>) -> PathBuf {
    ctx.shared.data_dir.join("agentos")
}

fn list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let root = root(ctx);
    let mut rels = vec!["AGENT.md".to_string()];
    let mut stack = vec![(root.join("memory"), "memory".to_string())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let (name, Ok(meta)) = (e.file_name().to_string_lossy().to_string(), e.metadata())
            else {
                continue;
            };
            let child = format!("{rel}/{name}");
            if meta.is_dir() {
                stack.push((e.path(), child));
            } else if meta.is_file() && allowed(&child) {
                rels.push(child);
            }
        }
    }
    rels.sort_by(|a, b| (a != "AGENT.md", a).cmp(&(b != "AGENT.md", b)));
    let mut items = Vec::new();
    for rel in rels {
        if let Ok(data) = std::fs::read(root.join(&rel))
            && no_symlinks(&root, &rel)
        {
            items.push(info(&rel, &data));
        }
    }
    Ok(Reply::json(200, Value::Array(items)))
}

/// Path from `?path=`: missing - 400, not on the allowlist - 403.
fn path_param(ctx: &Ctx<'_>) -> Result<String, ApiErr> {
    let raw = query_pairs(ctx.query)
        .into_iter()
        .find(|(k, _)| *k == "path")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiErr::new(400, "bad_request", "path: file path required"))?;
    let path = percent_decode(raw).ok_or_else(deny)?;
    if allowed(&path) && no_symlinks(&root(ctx), &path) {
        Ok(path)
    } else {
        Err(deny())
    }
}

fn read(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let rel = path_param(ctx)?;
    let data = std::fs::read(root(ctx).join(&rel)).map_err(|_| not_found())?;
    let mut reply = Reply::bytes(200, "text/plain; charset=utf-8", data.clone());
    reply
        .headers
        .push(("etag", format!("\"{}\"", rev_of(&data))));
    Ok(reply)
}

fn write(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let rel = path_param(ctx)?;
    let want = ctx
        .headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.trim()
                .trim_start_matches("W/")
                .trim_matches('"')
                .to_string()
        })
        .ok_or_else(|| {
            ApiErr::new(
                428,
                "precondition_required",
                "If-Match header with the file rev is required",
            )
        })?;
    let file = root(ctx).join(&rel);
    let cur = std::fs::read(&file)
        .ok()
        .map_or_else(|| "0".to_string(), |d| rev_of(&d));
    if cur != want {
        return Err(stale());
    }
    if ctx.body.len() > MAX_FILE {
        return Err(ApiErr::new(
            413,
            "payload_too_large",
            "File is larger than 256 KiB",
        ));
    }
    if std::str::from_utf8(ctx.body).is_err() {
        return Err(ApiErr::new(400, "bad_request", "File must be UTF-8 text"));
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    let tmp = file.with_extension("md.tmp");
    std::fs::write(&tmp, ctx.body).map_err(internal)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(internal)?;
    std::fs::rename(&tmp, &file).map_err(internal)?;
    let mut reply = Reply::json(200, info(&rel, ctx.body));
    reply
        .headers
        .push(("etag", format!("\"{}\"", rev_of(ctx.body))));
    Ok(reply)
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", "/files") => list(ctx),
        ("GET", _) => read(ctx),
        _ => write(ctx),
    }
}
