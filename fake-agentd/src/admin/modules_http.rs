//! `/modules` (api.md section 5.6): installed and catalog, install by `catalog_id`, `manifest` or `url`, edit, remove.
//! "Custom by URL" emulates the watcher check: only `https://modules.example.test/<name>.json`, an address with `evil` is rejected,
//! with `ask` it goes to confirmation (`202`, the module is not installed).

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::FrameType;
use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::modules_manifest::{PLACEMENTS, catalog, validate};
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::settings_http::invalid_fields;
use crate::state::Shared;
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
    r("GET", "/modules", Scope::Read),
    r("POST", "/modules", Scope::Admin),
    r("PATCH", "/modules/{module_id}", Scope::Admin),
    r("DELETE", "/modules/{module_id}", Scope::Admin),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

/// Host from which manifests are "served" in the mock.
const HOST: &str = "modules.example.test";
const MAX_CONFIG: usize = 16 * 1024;

fn module_json(
    manifest: &Value,
    installed: bool,
    enabled: bool,
    placement: &str,
    config: &Value,
) -> Value {
    json!({ "id": manifest["id"], "name": manifest["name"], "manifest": manifest, "installed": installed,
            "enabled": enabled, "placement": placement, "config": config })
}

fn installed_rows(ctx: &Ctx<'_>) -> Result<Vec<Value>, ApiErr> {
    installed_of(ctx.shared)
}

/// Installed modules only (for issuing a panel token).
pub fn installed(shared: &Shared) -> Result<Vec<Value>, ApiErr> {
    installed_of(shared)
}

fn installed_of(shared: &Shared) -> Result<Vec<Value>, ApiErr> {
    let conn = shared.db.reader().map_err(internal)?;
    let mut st = conn
        .prepare("SELECT id, name, manifest_json, enabled, placement, config_json FROM modules WHERE installed = 1 ORDER BY id")
        .map_err(internal)?;
    let rows = st
        .query_map([], |x| {
            let (manifest, config): (String, String) = (x.get(2)?, x.get(5)?);
            Ok(json!({
                "id": x.get::<_, String>(0)?, "name": x.get::<_, String>(1)?,
                "manifest": serde_json::from_str::<Value>(&manifest).unwrap_or(Value::Null),
                "installed": true, "enabled": x.get::<_, i64>(3)? != 0, "placement": x.get::<_, String>(4)?,
                "config": serde_json::from_str::<Value>(&config).unwrap_or_else(|_| json!({})),
            }))
        })
        .and_then(|r| r.collect::<Result<Vec<_>, _>>())
        .map_err(internal)?;
    Ok(rows)
}

fn one(ctx: &Ctx<'_>, id: &str) -> Result<Value, ApiErr> {
    installed_rows(ctx)?
        .into_iter()
        .find(|m| m["id"] == id)
        .ok_or_else(not_found)
}

fn publish(ctx: &Ctx<'_>, m: &Value) {
    ctx.shared.hub.emit("modules", FrameType::Delta, m.clone());
}

/// Installed modules and catalog entries: body of `GET /modules` and `items` of the `modules` topic snapshot.
pub fn all_items(shared: &Shared) -> Result<Vec<Value>, ApiErr> {
    let mut items = installed_of(shared)?;
    for m in catalog() {
        if !items.iter().any(|i| i["id"] == m["id"]) {
            let placement = m["placement"].as_str().unwrap_or("panel").to_string();
            items.push(module_json(&m, false, false, &placement, &json!({})));
        }
    }
    Ok(items)
}

fn list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    Ok(Reply::json(200, Value::Array(all_items(ctx.shared)?)))
}

/// Manifest by address: "watcher" check and "download" from the mock catalog.
fn manifest_by_url(url: &str) -> Result<Result<Value, ()>, ApiErr> {
    let bad = |why: &str| invalid("url", why);
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| bad("an https:// address is required"))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(['?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(bad("an address with login and password is not accepted"));
    }
    if authority != HOST {
        return Err(bad("could not download the manifest"));
    }
    if url.contains("evil") {
        return Err(bad("the watcher rejected the module"));
    }
    if url.contains("ask") {
        return Ok(Err(()));
    }
    let name = path.strip_suffix(".json").unwrap_or("");
    catalog()
        .into_iter()
        .find(|m| m["id"] == name)
        .map(Ok)
        .ok_or_else(|| bad("could not download the manifest"))
}

fn create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let body = object(ctx)?;
    let keys: Vec<&str> = ["manifest", "catalog_id", "url"]
        .into_iter()
        .filter(|k| body.contains_key(*k))
        .collect();
    if keys.len() != 1 {
        return Err(invalid(
            "manifest",
            "one of manifest, catalog_id, url is required",
        ));
    }
    let manifest = match keys[0] {
        "manifest" => body["manifest"].clone(),
        "catalog_id" => {
            let id = body["catalog_id"].as_str().unwrap_or_default();
            catalog()
                .into_iter()
                .find(|m| m["id"] == id)
                .ok_or_else(|| ApiErr::new(404, "not_found", "No such catalog entry"))?
        }
        _ => match manifest_by_url(body["url"].as_str().unwrap_or_default())? {
            Ok(m) => m,
            Err(()) => {
                return Ok(Reply::json(
                    202,
                    json!({ "id": ctx.shared.clock.now_unix(), "status": "pending", "kind": "module",
                            "detail": "The watcher is waiting for module install confirmation" }),
                ));
            }
        },
    };
    let errs = validate(&manifest);
    if !errs.is_empty() {
        return Err(invalid_fields(&errs));
    }
    let id = manifest["id"].as_str().unwrap_or_default();
    let placement = manifest["placement"].as_str().unwrap_or("panel");
    let name = manifest["name"].as_str().unwrap_or_default();
    let done = ctx
        .shared
        .db
        .write(|c| {
            let have: i64 = c.query_row(
                "SELECT count(*) FROM modules WHERE id = ?1 AND installed = 1",
                params![id],
                |x| x.get(0),
            )?;
            if have > 0 {
                return Ok(false);
            }
            c.execute(
                "INSERT INTO modules(id, name, manifest_json, installed, enabled, placement, config_json) VALUES (?1, ?2, ?3, 1, 1, ?4, '{}') \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, manifest_json = excluded.manifest_json, installed = 1, enabled = 1, placement = excluded.placement, config_json = '{}'",
                params![id, name, manifest.to_string(), placement],
            )?;
            Ok(true)
        })
        .map_err(internal)?;
    if !done {
        return Err(ApiErr::new(409, "conflict", "Module is already installed")
            .details(json!({ "reason": "already_installed" })));
    }
    let m = one(ctx, id)?;
    publish(ctx, &m);
    Ok(Reply::json(201, m))
}

fn module_id<'a>(ctx: &'a Ctx<'_>) -> &'a str {
    ctx.path.strip_prefix("/v1/modules/").unwrap_or("")
}

fn patch(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let id = module_id(ctx).to_string();
    one(ctx, &id)?;
    let m: Map<String, Value> = object(ctx)?;
    if let Some(k) = m
        .keys()
        .find(|k| !["enabled", "placement", "config"].contains(&k.as_str()))
    {
        return Err(invalid(k, "unknown field"));
    }
    let enabled = match m.get("enabled") {
        None => None,
        Some(Value::Bool(b)) => Some(i64::from(*b)),
        Some(_) => return Err(invalid("enabled", "true or false")),
    };
    let placement = match m.get("placement") {
        None => None,
        Some(Value::String(p)) if PLACEMENTS.contains(&p.as_str()) => Some(p.clone()),
        Some(_) => return Err(invalid("placement", "panel, widget or button")),
    };
    let config = match m.get("config") {
        None => None,
        Some(c @ Value::Object(_)) if c.to_string().len() <= MAX_CONFIG => Some(c.to_string()),
        Some(_) => return Err(invalid("config", "object up to 16 KiB")),
    };
    ctx.shared
        .db
        .write(|c| {
            c.execute(
                "UPDATE modules SET enabled = coalesce(?1, enabled), placement = coalesce(?2, placement), \
                 config_json = coalesce(?3, config_json) WHERE id = ?4 AND installed = 1",
                params![enabled, placement, config, id],
            )
        })
        .map_err(internal)?;
    let out = one(ctx, &id)?;
    publish(ctx, &out);
    Ok(Reply::json(200, out))
}

fn remove(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let id = module_id(ctx).to_string();
    let gone = ctx
        .shared
        .db
        .write(|c| {
            let had: Option<i64> = c
                .query_row(
                    "SELECT installed FROM modules WHERE id = ?1",
                    params![id],
                    |x| x.get(0),
                )
                .optional()?;
            if had != Some(1) {
                return Ok(false);
            }
            c.execute("DELETE FROM modules WHERE id = ?1", params![id])?;
            Ok(true)
        })
        .map_err(internal)?;
    if !gone {
        return Err(not_found());
    }
    ctx.shared.hub.emit(
        "modules",
        FrameType::Delta,
        json!({ "id": id, "installed": false }),
    );
    Ok(Reply::no_content())
}

pub fn dispatch(route: &Route, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", _) => list(ctx),
        ("POST", _) => create(ctx),
        ("PATCH", _) => patch(ctx),
        _ => remove(ctx),
    }
}
