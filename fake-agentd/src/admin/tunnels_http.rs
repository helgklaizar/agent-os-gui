//! `/tunnels` (api.md section 5.7): list, add, edit, delete; the `status` state is computed from `enabled`.

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::routes::Route;
use crate::store::Scope;

const fn r(method: &'static str, pattern: &'static str) -> Route {
    Route {
        method,
        pattern,
        scope: Scope::Admin,
        local_only: false,
    }
}

pub const ROUTES: &[Route] = &[
    r("GET", "/tunnels"),
    r("POST", "/tunnels"),
    r("PATCH", "/tunnels/{id}"),
    r("DELETE", "/tunnels/{id}"),
];

pub fn owns(route: &Route) -> bool {
    ROUTES
        .iter()
        .any(|x| x.method == route.method && x.pattern == route.pattern)
}

const TYPES: [&str; 5] = ["wireguard", "tailscale", "ssh", "cloudflare", "ngrok"];

fn row_json(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let mut st =
        conn.prepare("SELECT id, type, addr, descr, enabled FROM tunnels WHERE id = ?1")?;
    let mut rows = st.query_map(params![id], |x| {
        let on = x.get::<_, i64>(4)? != 0;
        Ok(json!({ "id": x.get::<_, i64>(0)?, "type": x.get::<_, String>(1)?, "addr": x.get::<_, String>(2)?,
                   "descr": x.get::<_, String>(3)?, "enabled": on, "status": if on { "up" } else { "down" } }))
    })?;
    rows.next().transpose()
}

/// Fields from the body; for `create` `type` and `addr` are required.
struct In {
    kind: Option<String>,
    addr: Option<String>,
    descr: Option<String>,
    enabled: Option<bool>,
}

fn parse(m: &Map<String, Value>, create: bool) -> Result<In, ApiErr> {
    let kind = match m.get("type") {
        Some(Value::String(t)) if TYPES.contains(&t.as_str()) => Some(t.clone()),
        None if !create => None,
        _ => {
            return Err(invalid(
                "type",
                "wireguard, tailscale, ssh, cloudflare or ngrok",
            ));
        }
    };
    let addr = match m.get("addr") {
        Some(Value::String(a)) if (1..=255).contains(&a.chars().count()) => Some(a.clone()),
        None if !create => None,
        _ => return Err(invalid("addr", "string of 1 to 255 characters")),
    };
    let descr = match m.get("descr") {
        None => None,
        Some(Value::String(d)) if d.chars().count() <= 200 => Some(d.clone()),
        Some(_) => return Err(invalid("descr", "string up to 200 characters")),
    };
    let enabled = match m.get("enabled") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(invalid("enabled", "true or false")),
    };
    Ok(In {
        kind,
        addr,
        descr,
        enabled,
    })
}

fn list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM tunnels ORDER BY id")
        .map_err(internal)?
        .query_map([], |x| x.get(0))
        .and_then(|r| r.collect::<Result<_, _>>())
        .map_err(internal)?;
    let mut items = Vec::new();
    for id in ids {
        items.extend(row_json(&conn, id).map_err(internal)?);
    }
    Ok(Reply::json(200, Value::Array(items)))
}

fn create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let p = parse(&object(ctx)?, true)?;
    let id = ctx
        .shared
        .db
        .write(|c| {
            c.execute(
                "INSERT INTO tunnels(type, addr, descr, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![
                    p.kind,
                    p.addr,
                    p.descr.unwrap_or_default(),
                    i64::from(p.enabled.unwrap_or(false))
                ],
            )?;
            Ok(c.last_insert_rowid())
        })
        .map_err(internal)?;
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let body = row_json(&conn, id)
        .map_err(internal)?
        .ok_or_else(not_found)?;
    Ok(Reply::json(201, body))
}

fn patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let p = parse(&object(ctx)?, false)?;
    let n = ctx
        .shared
        .db
        .write(|c| {
            c.execute(
                "UPDATE tunnels SET type = coalesce(?1, type), addr = coalesce(?2, addr), descr = coalesce(?3, descr), \
                 enabled = coalesce(?4, enabled) WHERE id = ?5",
                params![p.kind, p.addr, p.descr, p.enabled.map(i64::from), id],
            )
        })
        .map_err(internal)?;
    if n == 0 {
        return Err(not_found());
    }
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let body = row_json(&conn, id)
        .map_err(internal)?
        .ok_or_else(not_found)?;
    Ok(Reply::json(200, body))
}

pub fn dispatch(route: &Route, id: Option<i64>, ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    match (route.method, route.pattern) {
        ("GET", _) => list(ctx),
        ("POST", _) => create(ctx),
        ("PATCH", _) => patch(ctx, id.ok_or_else(not_found)?),
        _ => {
            let n = ctx
                .shared
                .db
                .write(|c| {
                    c.execute(
                        "DELETE FROM tunnels WHERE id = ?1",
                        params![id.unwrap_or(-1)],
                    )
                })
                .map_err(internal)?;
            if n == 0 {
                return Err(not_found());
            }
            Ok(Reply::no_content())
        }
    }
}
