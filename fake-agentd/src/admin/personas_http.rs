//! Personas (api.md section 5.6): `POST/PATCH/DELETE /personas`; routes and dispatcher are in `people_http`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};

use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::settings_http::emit;
use crate::tree;

const PERSONA_FIELDS: [&str; 12] = [
    "name",
    "color",
    "model_acc",
    "style",
    "lang",
    "signature",
    "mem_mode",
    "notif_mode",
    "work_hours",
    "quiet_hours",
    "account_ids",
    "id",
];

fn text_field(m: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
        Some(_) => Err(invalid(key, "string of allowed length")),
    }
}

fn enum_field(m: &Map<String, Value>, key: &str, ok: &[&str]) -> Result<Option<String>, ApiErr> {
    match m.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if ok.contains(&s.as_str()) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(key, &format!("allowed: {}", ok.join(", ")))),
    }
}

fn live_account(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT count(*) FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |x| x.get::<_, i64>(0),
    )
    .map(|n| n == 1)
}

/// Persona fields from the body: for `create` the name is required.
struct PersonaIn {
    sets: Vec<(&'static str, rusqlite::types::Value)>,
    accounts: Option<Vec<i64>>,
}

fn persona_in(ctx: &Ctx<'_>, create: bool) -> Result<PersonaIn, ApiErr> {
    let m = object(ctx)?;
    if let Some(k) = m.keys().find(|k| !PERSONA_FIELDS.contains(&k.as_str())) {
        return Err(invalid(k, "unknown field"));
    }
    let mut sets: Vec<(&'static str, rusqlite::types::Value)> = Vec::new();
    let text = |v: String| rusqlite::types::Value::Text(v);
    match m.get("name") {
        Some(Value::String(s)) if (1..=80).contains(&s.chars().count()) => {
            sets.push(("name", text(s.clone())));
        }
        None if !create => {}
        _ => return Err(invalid("name", "string of 1 to 80 characters")),
    }
    for (key, max) in [
        ("color", 32),
        ("style", 2000),
        ("signature", 500),
        ("work_hours", 64),
        ("quiet_hours", 64),
    ] {
        if let Some(v) = text_field(&m, key, max)? {
            sets.push((key, text(v)));
        }
    }
    if create && !m.contains_key("color") {
        sets.push(("color", text("#7c8aa5".into())));
    }
    for (key, ok) in [
        ("lang", &["ru", "en"][..]),
        ("mem_mode", &["own", "shared", "none"][..]),
        ("notif_mode", &["all", "important", "digest", "off"][..]),
    ] {
        if let Some(v) = enum_field(&m, key, ok)? {
            sets.push((key, text(v)));
        }
    }
    match m.get("model_acc") {
        None => {}
        Some(Value::Null) => sets.push(("model_acc", rusqlite::types::Value::Null)),
        Some(v) => {
            let id = v
                .as_i64()
                .ok_or_else(|| invalid("model_acc", "integer or null"))?;
            sets.push(("model_acc", rusqlite::types::Value::Integer(id)));
        }
    }
    let accounts = match m.get("account_ids") {
        None => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|v| {
                    v.as_i64()
                        .ok_or_else(|| invalid("account_ids", "list of integers"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(invalid("account_ids", "list of integers")),
    };
    Ok(PersonaIn { sets, accounts })
}

fn persona_json(ctx: &Ctx<'_>, id: i64) -> Result<Value, ApiErr> {
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let all = tree::personas_json(&conn).map_err(internal)?;
    all.into_iter()
        .find(|p| p["id"] == id)
        .ok_or_else(not_found)
}

/// Save a persona (`id = None` - create); returns the id or `Err(field)` on bad account references.
fn save_persona(
    ctx: &Ctx<'_>,
    id: Option<i64>,
    p: &PersonaIn,
) -> Result<Result<i64, &'static str>, ApiErr> {
    let before = tree::load(&ctx.shared.db.reader().map_err(internal)?).map_err(internal)?;
    let done = ctx
        .shared
        .db
        .write(|tx| {
            let t = tx.transaction()?;
            let all_ok = p
                .accounts
                .iter()
                .flatten()
                .try_fold(true, |ok, a| live_account(&t, *a).map(|x| ok && x))?;
            let model_ok = match p.sets.iter().find(|(k, _)| *k == "model_acc") {
                Some((_, rusqlite::types::Value::Integer(a))) => live_account(&t, *a)?,
                _ => true,
            };
            if !all_ok || !model_ok {
                return Ok(Err(if all_ok { "model_acc" } else { "account_ids" }));
            }
            let pid = match id {
                Some(pid) => {
                    let n: i64 = t.query_row(
                        "SELECT count(*) FROM personas WHERE id = ?1 AND deleted_at IS NULL",
                        params![pid],
                        |x| x.get(0),
                    )?;
                    if n == 0 {
                        return Ok(Err("not_found"));
                    }
                    pid
                }
                None => {
                    let name = p
                        .sets
                        .iter()
                        .find(|(k, _)| *k == "name")
                        .map(|(_, v)| v.clone());
                    let color = p
                        .sets
                        .iter()
                        .find(|(k, _)| *k == "color")
                        .map(|(_, v)| v.clone());
                    t.execute(
                        "INSERT INTO personas(name, color) VALUES (?1, ?2)",
                        params![name, color],
                    )?;
                    t.last_insert_rowid()
                }
            };
            for (col, v) in &p.sets {
                if id.is_none() && matches!(*col, "name" | "color") {
                    continue;
                }
                // column names come from the fixed list above, values are parameters
                t.execute(
                    &format!("UPDATE personas SET {col} = ?1 WHERE id = ?2"),
                    params![v, pid],
                )?;
            }
            if let Some(list) = &p.accounts {
                t.execute(
                    "DELETE FROM persona_accounts WHERE persona_id = ?1",
                    params![pid],
                )?;
                for a in list {
                    t.execute(
                        "INSERT OR IGNORE INTO persona_accounts VALUES (?1, ?2)",
                        params![pid, a],
                    )?;
                }
            }
            t.commit()?;
            Ok(Ok(pid))
        })
        .map_err(internal)?;
    if done.is_ok() {
        let after = tree::load(&ctx.shared.db.reader().map_err(internal)?).map_err(internal)?;
        crate::tree_pub::publish_diff(ctx.shared, &before, &after);
    }
    Ok(done)
}

pub(crate) fn create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let p = persona_in(ctx, true)?;
    match save_persona(ctx, None, &p)? {
        Ok(id) => Ok(Reply::json(201, persona_json(ctx, id)?)),
        Err(field) => Err(invalid(field, "account not found")),
    }
}

pub(crate) fn patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let p = persona_in(ctx, false)?;
    match save_persona(ctx, Some(id), &p)? {
        Ok(id) => Ok(Reply::json(200, persona_json(ctx, id)?)),
        Err("not_found") => Err(not_found()),
        Err(field) => Err(invalid(field, "account not found")),
    }
}

/// Persona deletion: sessions and folders lose the reference, `persona_accounts` go away, the balance priority is reset.
pub(crate) fn delete(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let before = tree::load(&ctx.shared.db.reader().map_err(internal)?).map_err(internal)?;
    let now = ctx.shared.clock.now_unix();
    let rev = ctx
        .shared
        .db
        .write(|c| {
            let tx = c.transaction()?;
            if tx.execute("DELETE FROM personas WHERE id = ?1", params![id])? == 0 {
                return Ok(None);
            }
            let page: Option<(String, i64)> = tx
                .query_row(
                    "SELECT value_json, rev FROM settings WHERE key = 'personas'",
                    [],
                    |x| Ok((x.get(0)?, x.get(1)?)),
                )
                .optional()?;
            let mut new_rev = None;
            if let Some((text, rev)) = page {
                let mut doc: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if doc.pointer("/balance/priority_persona_id").and_then(Value::as_i64) == Some(id)
                    && let Some(b) = doc.get_mut("balance").and_then(Value::as_object_mut)
                {
                    b.remove("priority_persona_id");
                    tx.execute(
                        "UPDATE settings SET value_json = ?1, rev = rev + 1, updated_at = ?2 WHERE key = 'personas'",
                        params![doc.to_string(), now],
                    )?;
                    new_rev = Some(rev + 1);
                }
            }
            tx.commit()?;
            Ok(Some(new_rev))
        })
        .map_err(internal)?;
    let Some(new_rev) = rev else {
        return Err(not_found());
    };
    let after = tree::load(&ctx.shared.db.reader().map_err(internal)?).map_err(internal)?;
    crate::tree_pub::publish_diff(ctx.shared, &before, &after);
    if let Some(rev) = new_rev {
        emit(ctx, "personas", rev);
    }
    Ok(Reply::no_content())
}
