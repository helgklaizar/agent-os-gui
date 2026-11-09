//! Chat routes: history and sending messages, stopping a reply, attachments (api.md section 5.2).
//!
//! `POST /sessions/{id}/messages` does not answer `202 ask`: a user sending to chat is not an agent `send` action.
//! Attachment size is bounded by the 1 MiB request body (`413 payload_too_large`).

use serde_json::{Map, Value, json};

use crate::chat_msgs::{self, NewMsg};
use crate::handlers::Ctx;
use crate::perm_store as store;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;
use crate::tree;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/sessions/{id}/messages", Scope::Read),
    r("POST", "/sessions/{id}/messages", Scope::Chat),
    r("POST", "/sessions/{id}/stop", Scope::Chat),
    r("POST", "/sessions/{id}/attachments", Scope::Chat),
    r("GET", "/attachments/{id}", Scope::Read),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

pub fn internal(e: impl std::fmt::Display) -> ApiErr {
    tracing::error!(error = %e, "fake-agentd storage failure");
    ApiErr::new(500, "internal", "Storage failure")
}

pub fn invalid(field: &str, why: &str) -> ApiErr {
    ApiErr::new(422, "validation_failed", why).details(json!({ "fields": [field] }))
}

pub fn not_found() -> ApiErr {
    ApiErr::new(404, "not_found", "Object not found")
}

pub fn object(ctx: &Ctx<'_>) -> Result<Map<String, Value>, ApiErr> {
    match serde_json::from_slice::<Value>(ctx.body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(ApiErr::new(
            400,
            "bad_request",
            "Request body: JSON object expected",
        )),
    }
}

pub fn query_pairs(query: Option<&str>) -> Vec<(&str, &str)> {
    query
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

pub fn int_param(v: &str, key: &str) -> Result<i64, ApiErr> {
    v.parse()
        .map_err(|_| ApiErr::new(400, "bad_request", &format!("{key}: integer expected")))
}

fn session_exists(ctx: &Ctx<'_>, id: i64) -> Result<(), ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let t = tree::load(&conn).map_err(internal)?;
    t.session(id).map(|_| ()).ok_or_else(not_found)
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let id = id.ok_or_else(not_found)?;
    match (route.method, route.pattern) {
        ("GET", "/sessions/{id}/messages") => list(ctx, id),
        ("POST", "/sessions/{id}/messages") => send(ctx, id),
        ("POST", "/sessions/{id}/stop") => {
            session_exists(ctx, id)?;
            crate::agent_sim::stop(ctx.shared, id);
            Ok(Reply::no_content())
        }
        ("POST", "/sessions/{id}/attachments") => upload(ctx, id),
        ("GET", "/attachments/{id}") => download(ctx, id),
        _ => Err(not_found()),
    }
}

fn list(ctx: &Ctx<'_>, session: i64) -> Result<Reply, ApiErr> {
    session_exists(ctx, session)?;
    let (mut before, mut limit) = (None, 50);
    for (k, v) in query_pairs(ctx.query) {
        match k {
            "before" => before = Some(int_param(v, k)?),
            "limit" => limit = int_param(v, k)?.clamp(1, 200),
            _ => {}
        }
    }
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let items = chat_msgs::page(&conn, session, before, limit).map_err(internal)?;
    Ok(Reply::json(200, Value::Array(items)))
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36 && uuid::Uuid::parse_str(s).is_ok()
}

fn attachment_ids(m: &Map<String, Value>) -> Result<Vec<i64>, ApiErr> {
    match m.get("attachment_ids") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_i64()
                    .ok_or_else(|| invalid("attachment_ids", "list of integers expected"))
            })
            .collect(),
        Some(_) => Err(invalid("attachment_ids", "list of integers expected")),
    }
}

enum Stored {
    New(Value),
    Existing(Value),
}

fn send(ctx: &Ctx<'_>, session: i64) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let body = m
        .get("body_md")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("body_md", "body_md is required"))?;
    let client = m
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|c| valid_uuid(c))
        .ok_or_else(|| invalid("client_id", "client_id: UUID"))?;
    let ids = attachment_ids(&m)?;
    session_exists(ctx, session)?;
    let metas: Vec<Value> = {
        let g = ctx.shared.chat.lock();
        ids.iter()
            .map(|i| {
                g.attachments.get(i).map(
                    |b| json!({ "id": i, "name": b.name, "size": b.data.len(), "mime": b.mime }),
                )
            })
            .collect::<Option<_>>()
            .ok_or_else(|| invalid("attachment_ids", "attachment not found"))?
    };
    if body.is_empty() && metas.is_empty() {
        return Err(invalid("body_md", "message is empty"));
    }
    let meta = (!metas.is_empty()).then(|| json!({ "attachments": metas }));
    let now = ctx.shared.clock.now_unix();
    let stored = ctx
        .shared
        .db
        .write(|w| {
            if let Some(old) = chat_msgs::by_client(w, session, client)? {
                return Ok(Stored::Existing(old));
            }
            let msg = NewMsg {
                session,
                client_id: Some(client),
                role: "user",
                author: None,
                body,
                status: "done",
                meta: meta.as_ref(),
            };
            let id = chat_msgs::insert(w, now, &msg)?;
            let persona = tree::effective_persona_id(w, session)?;
            let args = json!({ "len": body.len(), "attachments": metas.len() });
            let target = format!("session:{session}");
            store::log_action(
                w,
                now,
                "user",
                (Some(session), persona),
                ("send", Some(&target)),
                Some(&args),
                (Some("allow"), Some("ok")),
            )?;
            Ok(Stored::New(chat_msgs::get(w, id)?.unwrap_or(Value::Null)))
        })
        .map_err(internal)?;
    match stored {
        Stored::Existing(old) if old["body_md"] == body => Ok(Reply::json(200, old)),
        Stored::Existing(_) => Err(ApiErr::new(
            409,
            "client_id_conflict",
            "client_id is already taken by a different text",
        )),
        Stored::New(msg) => {
            chat_msgs::emit_created(ctx.shared, &msg);
            crate::agent_sim::on_user_message(ctx.shared, session, body.to_string());
            Ok(Reply::json(201, msg))
        }
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn header_param(head: &str, key: &str) -> Option<String> {
    let at = head.find(&format!("{key}=\""))? + key.len() + 2;
    let end = head[at..].find('"')?;
    Some(head[at..at + end].to_string())
}

/// The `file` part of `multipart/form-data`: name, type and content.
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Option<(String, String, Vec<u8>)> {
    let boundary = content_type
        .split("boundary=")
        .nth(1)?
        .trim()
        .trim_matches('"')
        .to_string();
    let delim = format!("--{boundary}").into_bytes();
    let mut pos = find(body, &delim, 0)?;
    while let Some(next) = find(body, &delim, pos + delim.len()) {
        let part = &body[pos + delim.len()..next];
        pos = next;
        let part = part.strip_prefix(b"\r\n").unwrap_or(part);
        let split = find(part, b"\r\n\r\n", 0)?;
        let head = String::from_utf8_lossy(&part[..split]).to_string();
        if !head.contains(" name=\"file\"") {
            continue;
        }
        let data = part[split + 4..]
            .strip_suffix(b"\r\n")
            .unwrap_or(&part[split + 4..])
            .to_vec();
        let mime = head
            .lines()
            .find_map(|l| {
                l.to_lowercase()
                    .starts_with("content-type:")
                    .then(|| l[13..].trim().to_string())
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        return Some((header_param(&head, "filename")?, mime, data));
    }
    None
}

fn upload(ctx: &Ctx<'_>, session: i64) -> Result<Reply, ApiErr> {
    session_exists(ctx, session)?;
    let ct = ctx
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct.starts_with("multipart/form-data") {
        return Err(invalid("file", "multipart/form-data expected"));
    }
    let (name, mime, data) = parse_multipart(ct, ctx.body)
        .ok_or_else(|| invalid("file", "the form has no file field"))?;
    let mut g = ctx.shared.chat.lock();
    g.next_attachment += 1;
    let id = g.next_attachment;
    let size = data.len();
    g.attachments.insert(
        id,
        crate::chat_state::Blob {
            name: name.clone(),
            mime: mime.clone(),
            data,
        },
    );
    Ok(Reply::json(
        201,
        json!({ "id": id, "name": name, "mime": mime, "size": size }),
    ))
}

fn download(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let g = ctx.shared.chat.lock();
    let b = g.attachments.get(&id).ok_or_else(not_found)?;
    Ok(Reply::bytes(
        200,
        "application/octet-stream",
        b.data.clone(),
    ))
}
