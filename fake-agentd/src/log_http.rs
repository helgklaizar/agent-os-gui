//! Reading the action log and watcher findings (api.md section 5.4): `{items, next_cursor}`, pages by `id` downward.

use rusqlite::params;
use serde_json::{Value, json};

use crate::chat_http::{int_param, internal, query_pairs};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};

struct Page {
    cursor: Option<i64>,
    limit: i64,
    filters: Vec<(String, String)>,
}

fn page_args(ctx: &Ctx<'_>, allowed: &[&str]) -> Result<Page, ApiErr> {
    let mut p = Page {
        cursor: None,
        limit: 50,
        filters: Vec::new(),
    };
    for (k, v) in query_pairs(ctx.query) {
        match k {
            "cursor" => p.cursor = Some(int_param(v, k)?),
            "limit" => p.limit = int_param(v, k)?.clamp(1, 200),
            "session_id" | "from" | "to" => {
                int_param(v, k)?;
                p.filters.push((k.to_string(), v.to_string()));
            }
            k if allowed.contains(&k) => p.filters.push((k.to_string(), v.to_string())),
            _ => {}
        }
    }
    Ok(p)
}

fn items_reply(items: Vec<Value>, limit: i64) -> Reply {
    let next = (items.len() as i64 == limit)
        .then(|| items.last().map(|i| i["id"].clone()))
        .flatten();
    Reply::json(200, json!({ "items": items, "next_cursor": next }))
}

fn filter<'a>(p: &'a Page, key: &str) -> Option<&'a str> {
    p.filters
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn actions(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let p = page_args(ctx, &["actor"])?;
    let num = |k: &str| filter(&p, k).and_then(|v| v.parse::<i64>().ok());
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let mut st = conn
        .prepare(
            "SELECT id, ts, actor, session_id, persona_id, action, target, args_json, decision, result, reversible, undone_at \
             FROM actions_log WHERE (?1 IS NULL OR session_id = ?1) AND (?2 IS NULL OR actor = ?2) \
             AND (?3 IS NULL OR ts >= ?3) AND (?4 IS NULL OR ts <= ?4) AND (?5 IS NULL OR id < ?5) ORDER BY id DESC LIMIT ?6",
        )
        .map_err(internal)?;
    let rows = st
        .query_map(params![num("session_id"), filter(&p, "actor"), num("from"), num("to"), p.cursor, p.limit], |r| {
            let args: Option<String> = r.get(7)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "ts": r.get::<_, i64>(1)?, "actor": r.get::<_, String>(2)?,
                "session_id": r.get::<_, Option<i64>>(3)?, "persona_id": r.get::<_, Option<i64>>(4)?,
                "action": r.get::<_, String>(5)?, "target": r.get::<_, Option<String>>(6)?,
                "args": args.and_then(|a| serde_json::from_str::<Value>(&a).ok()),
                "decision": r.get::<_, Option<String>>(8)?, "result": r.get::<_, Option<String>>(9)?,
                "reversible": r.get::<_, i64>(10)? != 0, "undone_at": r.get::<_, Option<i64>>(11)?,
            }))
        })
        .map_err(internal)?;
    Ok(items_reply(
        rows.collect::<Result<_, _>>().map_err(internal)?,
        p.limit,
    ))
}

pub fn findings(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let p = page_args(ctx, &["kind", "verdict"])?;
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let mut st = conn
        .prepare(
            "SELECT id, ts, kind, severity, ref_table, ref_id, verdict, detail FROM observer_findings \
             WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR verdict = ?2) AND (?3 IS NULL OR id < ?3) ORDER BY id DESC LIMIT ?4",
        )
        .map_err(internal)?;
    let rows = st
        .query_map(params![filter(&p, "kind"), filter(&p, "verdict"), p.cursor, p.limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "ts": r.get::<_, i64>(1)?, "kind": r.get::<_, String>(2)?,
                "severity": r.get::<_, String>(3)?, "ref_table": r.get::<_, Option<String>>(4)?,
                "ref_id": r.get::<_, Option<String>>(5)?, "verdict": r.get::<_, String>(6)?,
                "detail": r.get::<_, Option<String>>(7)?,
            }))
        })
        .map_err(internal)?;
    Ok(items_reply(
        rows.collect::<Result<_, _>>().map_err(internal)?,
        p.limit,
    ))
}
