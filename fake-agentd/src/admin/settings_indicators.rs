//! Page `GET`/`PATCH /v1/settings/indicators`: enabling and ordering of bar indicators (the `indicators` table);
//! `rev` is the `settings` row with key `indicators`.

use rusqlite::params;
use serde_json::{Map, Value, json};

use crate::chat_http::{internal, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::settings_conn::FieldErr;
use crate::settings_http::{Outcome, emit, if_match, invalid_fields, page_reply, row, stale};

const IND_KEY: &str = "indicators";

pub(crate) fn doc(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let rev = row(&conn, IND_KEY).map_err(internal)?.map_or(0, |(_, r)| r);
    let mut st = conn
        .prepare("SELECT id, enabled, position, config_json FROM indicators ORDER BY position, id")
        .map_err(internal)?;
    let items: Vec<Value> = st
        .query_map([], |x| {
            let cfg: String = x.get(3)?;
            let mut item = json!({
                "id": x.get::<_, String>(0)?, "enabled": x.get::<_, i64>(1)? != 0,
                "position": x.get::<_, i64>(2)?,
            });
            // `config?` is optional (api.md section 5.7): an empty one is not returned, as in the core.
            let cfg = serde_json::from_str::<Value>(&cfg).unwrap_or_else(|_| json!({}));
            if cfg.as_object().is_some_and(|m| !m.is_empty()) {
                item["config"] = cfg;
            }
            Ok(item)
        })
        .and_then(|rows| rows.collect::<Result<_, _>>())
        .map_err(internal)?;
    Ok(page_reply(json!({ "items": items }), rev))
}

fn item_errors(items: &Value, known: &[String]) -> Vec<FieldErr> {
    let Some(list) = items.as_array() else {
        return vec![("items".into(), "array expected".into())];
    };
    let mut errs = Vec::new();
    for (i, it) in list.iter().enumerate() {
        let f = |n: &str| format!("items[{i}].{n}");
        let Some(o) = it.as_object() else {
            errs.push((format!("items[{i}]"), "object expected".into()));
            continue;
        };
        match o.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() && known.iter().any(|k| k == id) => {}
            Some(id) if !id.is_empty() => errs.push((f("id"), "unknown indicator".into())),
            _ => errs.push((f("id"), "a non-empty string is required".into())),
        }
        if !o.get("enabled").is_some_and(Value::is_boolean) {
            errs.push((f("enabled"), "true or false is required".into()));
        }
        if o.get("position").and_then(Value::as_u64).is_none() {
            errs.push((f("position"), "a non-negative integer is required".into()));
        }
        if o.get("config")
            .is_some_and(|c| !c.is_object() && !c.is_null())
        {
            errs.push((f("config"), "object expected".into()));
        }
    }
    errs
}

pub(crate) fn patch(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let want = if_match(ctx)?;
    let body: Map<String, Value> = object(ctx)?;
    let now = ctx.shared.clock.now_unix();
    let out = ctx
        .shared
        .db
        .write(|tx| {
            let rev = row(tx, IND_KEY)?.map_or(0, |(_, r)| r);
            if rev != want {
                return Ok(Outcome::Stale);
            }
            let known: Vec<String> = tx
                .prepare("SELECT id FROM indicators")?
                .query_map([], |x| x.get(0))?
                .collect::<Result<_, _>>()?;
            let items = body.get("items").cloned().unwrap_or_else(|| json!([]));
            let errs = item_errors(&items, &known);
            if !errs.is_empty() {
                return Ok(Outcome::Invalid(errs));
            }
            for it in items.as_array().into_iter().flatten() {
                tx.execute(
                    "UPDATE indicators SET enabled = ?1, position = ?2, config_json = COALESCE(?3, config_json) WHERE id = ?4",
                    params![
                        i64::from(it["enabled"].as_bool().unwrap_or(true)),
                        it["position"].as_i64().unwrap_or(0),
                        it.get("config").filter(|c| c.is_object()).map(Value::to_string),
                        it["id"].as_str().unwrap_or(""),
                    ],
                )?;
            }
            tx.execute(
                "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, '{}', 1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET rev = settings.rev + 1, updated_at = excluded.updated_at",
                params![IND_KEY, now],
            )?;
            Ok(Outcome::Saved(Value::Null, rev + 1))
        })
        .map_err(internal)?;
    match out {
        Outcome::Stale => Err(stale()),
        Outcome::Invalid(e) => Err(invalid_fields(&e)),
        Outcome::Saved(_, rev) => {
            crate::bar::publish_tables(ctx.shared, &ctx.shared.db).map_err(internal)?;
            emit(ctx, IND_KEY, rev);
            doc(ctx)
        }
    }
}
