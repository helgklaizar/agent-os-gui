//! Bar data: fixtures of accounts, quotas and indicators in SQLite, building the `bar.quota`, `bar.indicators`, `bar.weather` frames.
//!
//! Live metrics (`bar.sys`) are not written to the DB (db.md section 3.12) and come from `sim`. Fixtures are created at mock start, not by a migration.

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::FrameType;
use crate::db::{Db, DbError};
use crate::state::Shared;

struct Account {
    slug: &'static str,
    kind: &'static str,
    label: &'static str,
    secret_ref: Option<&'static str>,
}

const ACCOUNTS: [Account; 3] = [
    Account {
        slug: "cl1",
        kind: "anthropic",
        label: "Claude · key-1",
        secret_ref: Some("ANTHROPIC_KEY_1"),
    },
    Account {
        slug: "cl2",
        kind: "anthropic",
        label: "Claude · key-2",
        secret_ref: Some("ANTHROPIC_KEY_2"),
    },
    Account {
        slug: "ol1",
        kind: "ollama",
        label: "Ollama · local",
        secret_ref: None,
    },
];

const INDICATORS: [&str; 8] = [
    "cpu",
    "gpu",
    "ram",
    "disk",
    "temp",
    "quota.cl1",
    "quota.cl2",
    "quota.ol1",
];

fn seed_rows(w: &mut Connection, now: i64) -> rusqlite::Result<()> {
    let tx = w.transaction()?;
    let empty: i64 = tx.query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))?;
    if empty == 0 {
        for a in &ACCOUNTS {
            tx.execute(
                "INSERT INTO accounts(slug, type, label, secret_ref, status, created_at) VALUES (?1, ?2, ?3, ?4, 'connected', ?5)",
                params![a.slug, a.kind, a.label, a.secret_ref, now],
            )?;
        }
        let id = |slug: &str| -> rusqlite::Result<i64> {
            tx.query_row(
                "SELECT id FROM accounts WHERE slug = ?1",
                params![slug],
                |r| r.get(0),
            )
        };
        // cl1: used 0.62, rate 1.4, 5 h window, reset in 97 min (ETA ~ 81 min before reset); cl2: used 0.18, rate 0.4
        let api = "INSERT INTO quotas(account_id, kind, window_h, used, rate, usage_tokens, usage_cost, limit_requests, limit_tokens, remaining, reset_at, updated_at) VALUES (?1, 'api_key', 5, ?2, ?3, ?4, ?5, 4000, 2000000, ?6, ?7, ?8)";
        tx.execute(
            api,
            params![
                id("cl1")?,
                0.62,
                1.4,
                120_000,
                1.84,
                0.38,
                now + 97 * 60,
                now
            ],
        )?;
        tx.execute(
            api,
            params![
                id("cl2")?,
                0.18,
                0.4,
                41_000,
                0.52,
                0.82,
                now + 180 * 60,
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO quotas(account_id, kind, window_h, load, updated_at) VALUES (?1, 'local', 0, 0.22, ?2)",
            params![id("ol1")?, now],
        )?;
        for slug in ["cl1", "cl2"] {
            for (k, used) in [(2, 0.2), (1, 0.4), (0, 0.5)] {
                tx.execute(
                    "INSERT INTO quota_samples(account_id, ts, used, usage_tokens, cost) VALUES (?1, ?2, ?3, 1000, 0.01)",
                    params![id(slug)?, now - k * 600, used],
                )?;
            }
        }
    }
    for (position, name) in INDICATORS.iter().enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO indicators(id, position) VALUES (?1, ?2)",
            params![name, position as i64],
        )?;
    }
    // `personas` page (balance): the default value is already written, `rev = 1` (reads `balance.fallback`).
    tx.execute(
        "INSERT OR IGNORE INTO settings(key, value_json, rev, updated_at) VALUES ('personas', ?1, 1, ?2)",
        params![r#"{"balance":{"fallback":"local"}}"#, now],
    )?;
    tx.commit()
}

/// Creates fixtures if the accounts table is empty (leaves existing data alone).
pub fn seed(db: &Db, now: i64) -> Result<(), DbError> {
    db.write(|w| seed_rows(w, now))
}

/// Weather fixture: the live source is the core or a module, the values here are only mock data.
pub fn weather(now: i64) -> Value {
    json!({ "temp_c": 24, "icon": "sun", "text": "clear", "place": "Jerusalem", "updated_at": now })
}

fn limit(limit: Option<i64>, share: Option<f64>, reset: Option<i64>) -> Value {
    match limit {
        Some(l) => json!({
            "limit": l,
            "remaining": (l as f64 * share.unwrap_or(0.0)).round() as i64,
            "reset_at": reset,
        }),
        None => Value::Null,
    }
}

/// Whether it runs out before it resets: `ETA = (1 - used) / (rate / window_h)` hours.
fn eta_at(
    now: i64,
    used: Option<f64>,
    rate: Option<f64>,
    window: Option<f64>,
    reset: Option<i64>,
) -> Option<i64> {
    let (used, rate, window, reset) = (used?, rate?, window?, reset?);
    if rate <= 1.0 || window <= 0.0 {
        return None;
    }
    let eta = now + ((1.0 - used) / (rate / window) * 3600.0) as i64;
    (eta < reset).then_some(eta)
}

/// Quota frames from the tables. The `slug`, `label`, `provider` fields are a mock extension: `Quota` in openapi has none of them.
pub fn quota_items(db: &Db, now: i64) -> Result<Vec<Value>, DbError> {
    let conn = db.reader()?;
    let mut stmt = conn.prepare(
        "SELECT a.id, a.slug, a.type, a.label, q.kind, q.window_h, q.used, q.rate, q.load, q.usage_tokens, q.usage_cost, \
         q.limit_requests, q.limit_tokens, q.remaining, q.reset_at, q.updated_at \
         FROM quotas q JOIN accounts a ON a.id = q.account_id WHERE a.deleted_at IS NULL ORDER BY a.id",
    )?;
    let rows = stmt.query_map([], |r| {
        let (window, used, rate): (Option<f64>, Option<f64>, Option<f64>) = (r.get(5)?, r.get(6)?, r.get(7)?);
        let (tokens, cost): (Option<i64>, Option<f64>) = (r.get(9)?, r.get(10)?);
        let (lim_req, lim_tok, share, reset): (Option<i64>, Option<i64>, Option<f64>, Option<i64>) =
            (r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?);
        let usage = if tokens.is_some() || cost.is_some() {
            json!({ "period": "day", "tokens_in": tokens.unwrap_or(0), "tokens_out": 0, "cost": cost.unwrap_or(0.0), "currency": "USD" })
        } else {
            Value::Null
        };
        let limits = if lim_req.is_some() || lim_tok.is_some() {
            json!({ "requests_per_min": limit(lim_req, share, reset), "tokens_per_min": limit(lim_tok, share, reset) })
        } else {
            Value::Null
        };
        Ok(json!({
            "account_id": r.get::<_, i64>(0)?, "slug": r.get::<_, String>(1)?, "provider": r.get::<_, String>(2)?,
            "label": r.get::<_, String>(3)?, "kind": r.get::<_, String>(4)?,
            "window_h": window, "used": used, "rate": rate, "reset_at": reset,
            "eta_at": eta_at(now, used, rate, window, reset),
            "load": r.get::<_, Option<f64>>(8)?, "usage": usage, "limits": limits, "updated_at": r.get::<_, i64>(15)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Indicator states (`indicators`); live metric values do not get here.
pub fn indicator_items(db: &Db) -> Result<Vec<Value>, DbError> {
    let conn = db.reader()?;
    let mut stmt = conn.prepare(
        "SELECT id, enabled, position, last_state, transition_seq, last_change_at FROM indicators ORDER BY position, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "id": r.get::<_, String>(0)?, "enabled": r.get::<_, i64>(1)? != 0, "position": r.get::<_, i64>(2)?,
            "state": r.get::<_, String>(3)?, "transition_seq": r.get::<_, i64>(4)?, "last_change_at": r.get::<_, Option<i64>>(5)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Re-reads the tables and publishes the `bar.quota` and `bar.indicators` snapshots (the topic's last state for new subscribers).
pub fn publish_tables(shared: &Shared, db: &Db) -> Result<(), DbError> {
    let now = shared.clock.now_unix();
    shared.hub.emit(
        "bar.quota",
        FrameType::Snapshot,
        json!({ "items": quota_items(db, now)? }),
    );
    shared.hub.emit(
        "bar.indicators",
        FrameType::Snapshot,
        json!({ "items": indicator_items(db)? }),
    );
    Ok(())
}

/// Fixtures and first snapshots at startup.
pub fn start(shared: &Shared, db: &Db) -> Result<(), DbError> {
    let now = shared.clock.now_unix();
    seed(db, now)?;
    publish_tables(shared, db)?;
    shared
        .hub
        .emit("bar.weather", FrameType::Snapshot, weather(now));
    Ok(())
}
