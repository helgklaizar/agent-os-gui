//! Accounts (api.md section 5.6): `GET/POST/PATCH /accounts` and connecting; routes and dispatcher are in `people_http`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::chat_http::{internal, invalid, not_found, object};
use crate::handlers::Ctx;
use crate::reply::{ApiErr, Reply};
use crate::state::Shared;

pub(crate) fn account_json(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT id, type, label, status, created_at FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |x| {
            Ok(json!({ "id": x.get::<_, i64>(0)?, "type": x.get::<_, String>(1)?, "label": x.get::<_, String>(2)?,
                       "status": x.get::<_, String>(3)?, "created_at": x.get::<_, i64>(4)? }))
        },
    )
    .optional()
}

/// All accounts: body of `GET /accounts` and `items` of the `connections` topic snapshot.
pub fn all_items(shared: &Shared) -> Result<Vec<Value>, ApiErr> {
    let conn = shared.db.reader().map_err(internal)?;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM accounts WHERE deleted_at IS NULL ORDER BY id")
        .map_err(internal)?
        .query_map([], |x| x.get(0))
        .and_then(|r| r.collect::<Result<_, _>>())
        .map_err(internal)?;
    let mut items = Vec::new();
    for id in ids {
        items.extend(account_json(&conn, id).map_err(internal)?);
    }
    Ok(items)
}

pub(crate) fn list(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    Ok(Reply::json(200, Value::Array(all_items(ctx.shared)?)))
}

fn label_of(m: &Map<String, Value>) -> Result<Option<String>, ApiErr> {
    match m.get("label") {
        None => Ok(None),
        Some(Value::String(s)) if (1..=80).contains(&s.chars().count()) => Ok(Some(s.clone())),
        Some(_) => Err(invalid("label", "string of 1 to 80 characters")),
    }
}

pub(crate) fn create(ctx: &Ctx<'_>) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let kind = m
        .get("type")
        .and_then(Value::as_str)
        .filter(|t| {
            (1..=32).contains(&t.len())
                && t.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .ok_or_else(|| {
            invalid(
                "type",
                "lowercase Latin letters, digits, \"_\", up to 32 characters",
            )
        })?;
    let label = label_of(&m)?.ok_or_else(|| invalid("label", "string of 1 to 80 characters"))?;
    let now = ctx.shared.clock.now_unix();
    let id = ctx
        .shared
        .db
        .write(|c| {
            let tx = c.transaction()?;
            let n: i64 =
                tx.query_row("SELECT coalesce(max(id), 0) + 1 FROM accounts", [], |x| {
                    x.get(0)
                })?;
            let slug = format!("u{n}-{kind}");
            tx.execute(
                "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![slug, kind, label, now],
            )?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            Ok(id)
        })
        .map_err(internal)?;
    let conn = ctx.shared.db.reader().map_err(internal)?;
    let body = account_json(&conn, id)
        .map_err(internal)?
        .ok_or_else(not_found)?;
    Ok(Reply::json(201, body))
}

pub(crate) fn patch(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let m = object(ctx)?;
    let label = label_of(&m)?;
    let n = ctx
        .shared
        .db
        .write(|c| {
            c.execute(
                "UPDATE accounts SET label = coalesce(?1, label) WHERE id = ?2 AND deleted_at IS NULL",
                params![label, id],
            )
        })
        .map_err(internal)?;
    if n == 0 {
        return Err(not_found());
    }
    let conn = ctx.shared.db.reader().map_err(internal)?;
    Ok(Reply::json(
        200,
        account_json(&conn, id)
            .map_err(internal)?
            .ok_or_else(not_found)?,
    ))
}

pub(crate) fn connect(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let kind: String = {
        let conn = ctx.shared.db.reader().map_err(internal)?;
        let acc = account_json(&conn, id)
            .map_err(internal)?
            .ok_or_else(not_found)?;
        acc["type"].as_str().unwrap_or_default().to_string()
    };
    let flow = match kind.as_str() {
        "wa" => "qr",
        "gmail" => "oauth",
        _ => "token",
    };
    if flow == "qr" {
        ctx.shared
            .db
            .write(|c| {
                c.execute(
                    "UPDATE accounts SET status = 'qr' WHERE id = ?1",
                    params![id],
                )
            })
            .map_err(internal)?;
        crate::bar::publish_tables(ctx.shared, &ctx.shared.db).map_err(internal)?;
    }
    let now = ctx.shared.clock.now_unix();
    Ok(Reply::json(
        202,
        json!({
            "account_id": id, "kind": flow, "status": "pending",
            "url": (flow == "oauth").then(|| format!("https://auth.example.test/oauth?account={id}")),
            "qr_data": (flow == "qr").then(|| format!("FAKE-QR-{id}")),
            "expires_at": now + 300,
        }),
    ))
}

pub(crate) fn disconnect(ctx: &Ctx<'_>, id: i64) -> Result<Reply, ApiErr> {
    let n = ctx
        .shared
        .db
        .write(|c| {
            c.execute(
                "UPDATE accounts SET status = CASE WHEN status = 'qr' THEN 'disconnected' ELSE status END WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
            )
        })
        .map_err(internal)?;
    if n == 0 {
        return Err(not_found());
    }
    crate::bar::publish_tables(ctx.shared, &ctx.shared.db).map_err(internal)?;
    Ok(Reply::no_content())
}
