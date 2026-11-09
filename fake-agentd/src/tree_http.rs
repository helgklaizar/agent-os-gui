//! Tree routes (folders, sessions, threads, read-only personas): table and handlers.

use serde_json::{Map, Value, json};

use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;
use crate::tree::{self, Tree};
use crate::tree_ops::{self as ops, FolderPatch, SessionPatch, TreeError};
use crate::tree_session_ops as sops;

const fn r(method: &'static str, pattern: &'static str, scope: Scope) -> Route {
    Route {
        method,
        pattern,
        scope,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/folders", Scope::Read),
    r("POST", "/folders", Scope::Chat),
    r("PATCH", "/folders/{id}", Scope::Chat),
    r("DELETE", "/folders/{id}", Scope::Chat),
    r("GET", "/sessions", Scope::Read),
    r("POST", "/sessions", Scope::Chat),
    r("GET", "/sessions/{id}", Scope::Read),
    r("PATCH", "/sessions/{id}", Scope::Chat),
    r("DELETE", "/sessions/{id}", Scope::Chat),
    r("POST", "/sessions/{id}/threads", Scope::Chat),
    r("POST", "/sessions/{id}/read", Scope::Chat),
    r("GET", "/personas", Scope::Read),
];

fn internal(e: impl std::fmt::Display) -> ApiErr {
    tracing::error!(error = %e, "fake-agentd storage failure");
    ApiErr::new(500, "internal", "Storage failure")
}

impl From<TreeError> for ApiErr {
    fn from(e: TreeError) -> ApiErr {
        match e {
            TreeError::NotFound => ApiErr::new(404, "not_found", "Object not found"),
            TreeError::Invalid(field, why) => {
                ApiErr::new(422, "validation_failed", why).details(json!({ "fields": [field] }))
            }
            TreeError::Conflict(why) => ApiErr::new(409, "conflict", why),
            TreeError::Db(m) => internal(m),
        }
    }
}

fn invalid(field: &str, why: &str) -> ApiErr {
    ApiErr::new(422, "validation_failed", why).details(json!({ "fields": [field] }))
}

fn object(ctx: &Ctx<'_>) -> Result<Map<String, Value>, ApiErr> {
    match serde_json::from_slice::<Value>(ctx.body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(ApiErr::new(
            400,
            "bad_request",
            "Request body: JSON object expected",
        )),
    }
}

fn required_name(m: &Map<String, Value>) -> Result<String, ApiErr> {
    m.get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid("name", "name is required"))
}

fn opt_str(m: &Map<String, Value>, key: &str) -> Result<Option<String>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(key, "string expected")),
    }
}

fn opt_bool(m: &Map<String, Value>, key: &str) -> Result<Option<bool>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(invalid(key, "true or false expected")),
    }
}

/// Key missing - `None`; `null` - `Some(None)`; an integer - `Some(Some(n))`.
fn nullable_int(m: &Map<String, Value>, key: &str) -> Result<Option<Option<i64>>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v
            .as_i64()
            .map(|n| Some(Some(n)))
            .ok_or_else(|| invalid(key, "integer or null expected")),
    }
}

fn nullable_str(m: &Map<String, Value>, key: &str) -> Result<Option<Option<String>>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(invalid(key, "string or null expected")),
    }
}

fn read_tree(ctx: &Ctx<'_>) -> Result<Tree, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    tree::load(&conn).map_err(internal)
}

fn session_reply(ctx: &Ctx<'_>, id: i64, status: u16) -> Result<Reply, ApiErr> {
    let t = read_tree(ctx)?;
    let s = t.session(id).ok_or(TreeError::NotFound)?;
    Ok(Reply::json(status, t.session_json(s)))
}

fn folder_reply(ctx: &Ctx<'_>, id: i64, status: u16) -> Result<Reply, ApiErr> {
    let t = read_tree(ctx)?;
    let f = t.folder(id).ok_or(TreeError::NotFound)?;
    Ok(Reply::json(status, t.folder_json(f)))
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let none = || ApiErr::new(404, "not_found", "Object not found");
    match (route.method, route.pattern) {
        ("GET", "/folders") => Ok(Reply::json(
            200,
            Value::Array(read_tree(ctx)?.folders_json()),
        )),
        ("POST", "/folders") => folder_create(ctx),
        ("PATCH", "/folders/{id}") => folder_patch(ctx, id.ok_or_else(none)?),
        ("DELETE", "/folders/{id}") => {
            ops::delete_folder(ctx.shared, id.ok_or_else(none)?)?;
            Ok(Reply::no_content())
        }
        ("GET", "/sessions") => sessions_list(ctx),
        ("POST", "/sessions") => session_create(ctx),
        ("GET", "/sessions/{id}") => session_reply(ctx, id.ok_or_else(none)?, 200),
        ("PATCH", "/sessions/{id}") => session_patch(ctx, id.ok_or_else(none)?),
        ("DELETE", "/sessions/{id}") => {
            sops::delete_session(ctx.shared, id.ok_or_else(none)?)?;
            Ok(Reply::no_content())
        }
        ("POST", "/sessions/{id}/threads") => thread_create(ctx, id.ok_or_else(none)?),
        ("POST", "/sessions/{id}/read") => {
            sops::mark_read(ctx.shared, id.ok_or_else(none)?)?;
            Ok(Reply::no_content())
        }
        ("GET", "/personas") => {
            let conn = ctx.shared.db.reader().map_err(internal)?;
            Ok(Reply::json(
                200,
                Value::Array(tree::personas_json(&conn).map_err(internal)?),
            ))
        }
        _ => Err(none()),
    }
}

fn folder_create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let name = required_name(&m)?;
    let parent = nullable_int(&m, "parent_id")?.flatten();
    let persona = nullable_int(&m, "persona_id")?.flatten();
    let id = ops::create_folder(ctx.shared, &name, parent, persona)?;
    folder_reply(ctx, id, 201)
}

fn folder_patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let patch = FolderPatch {
        name: opt_str(&m, "name")?,
        parent: nullable_int(&m, "parent_id")?,
        persona: nullable_int(&m, "persona_id")?,
        open: opt_bool(&m, "open")?,
    };
    ops::patch_folder(ctx.shared, id, patch)?;
    folder_reply(ctx, id, 200)
}

fn query_pairs(query: Option<&str>) -> Vec<(&str, &str)> {
    query
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

fn int_param(v: &str, key: &str) -> Result<i64, ApiErr> {
    v.parse()
        .map_err(|_| ApiErr::new(400, "bad_request", &format!("{key}: integer expected")))
}

/// `?folder_id`, `?parent_id`, `?archived` (archived ones are hidden by default).
fn sessions_list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let (mut folder, mut parent, mut archived) = (None, None, false);
    for (k, v) in query_pairs(ctx.query) {
        match k {
            "folder_id" => folder = Some(int_param(v, k)?),
            "parent_id" => parent = Some(int_param(v, k)?),
            "archived" => {
                archived = match v {
                    "true" => true,
                    "false" => false,
                    _ => return Err(ApiErr::new(400, "bad_request", "archived: true or false")),
                }
            }
            _ => {}
        }
    }
    let t = read_tree(ctx)?;
    let items = t
        .sessions
        .iter()
        .filter(|s| s.archived == archived)
        .filter(|s| folder.is_none_or(|f| s.folder_id == Some(f)))
        .filter(|s| parent.is_none_or(|p| s.parent_id == Some(p)))
        .map(|s| t.session_json(s))
        .collect();
    Ok(Reply::json(200, Value::Array(items)))
}

fn session_create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let name = required_name(&m)?;
    let folder = nullable_int(&m, "folder_id")?.flatten();
    let persona = nullable_int(&m, "persona_id")?.flatten();
    let path = nullable_str(&m, "path")?.flatten();
    let id = sops::create_session(ctx.shared, &name, folder, persona, path)?;
    session_reply(ctx, id, 201)
}

fn session_patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let cfg = match m.get("cfg") {
        None => None,
        Some(Value::Object(c)) => Some(c.clone()),
        Some(_) => return Err(invalid("cfg", "cfg: object expected")),
    };
    let patch = SessionPatch {
        name: opt_str(&m, "name")?,
        folder: nullable_int(&m, "folder_id")?,
        persona: nullable_int(&m, "persona_id")?,
        path: nullable_str(&m, "path")?,
        cfg,
        archived: opt_bool(&m, "archived")?,
    };
    sops::patch_session(ctx.shared, id, patch)?;
    session_reply(ctx, id, 200)
}

fn thread_create(ctx: &Ctx<'_>, parent: i64) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let name = required_name(&m)?;
    let id = sops::create_thread(ctx.shared, parent, &name)?;
    session_reply(ctx, id, 201)
}
