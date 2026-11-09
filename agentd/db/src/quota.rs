//! Quotas by fact (`QuotaStore`, A3-10; docs/reference/db.md section 3.5, Q1): consumption is computed from `messages.meta.usage`, limits from the
//! provider response headers. The subscription remainder is not stored. Session accounting is a query over `messages.meta` (DB schema unchanged).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentd_core::{
    Clock, LimitWindow, PortFuture, QuotaStore, RateLimit, UsageRecord, WINDOW_SECS, eta_at,
    rate_of, tightest, used_fraction,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::Db;

const DAY_SECS: i64 = 86_400;

pub struct QuotaRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    /// Latest limits per account: exact request and token windows (the `quotas` columns hold only the tightest one).
    windows: Mutex<HashMap<i64, RateLimit>>,
}

impl QuotaRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> QuotaRepo {
        QuotaRepo {
            db,
            clock,
            windows: Mutex::new(HashMap::new()),
        }
    }

    fn windows(&self) -> std::sync::MutexGuard<'_, HashMap<i64, RateLimit>> {
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Sums {
    tokens_in: i64,
    tokens_out: i64,
    cost: Option<f64>,
    currency: Option<String>,
}

/// Account consumption since `since`; `session` restricts it to one session.
fn sums(c: &Connection, account: i64, since: i64, session: Option<i64>) -> rusqlite::Result<Sums> {
    c.query_row(
        "SELECT COALESCE(SUM(json_extract(meta_json,'$.usage.tokens_in')),0),
                COALESCE(SUM(json_extract(meta_json,'$.usage.tokens_out')),0),
                SUM(json_extract(meta_json,'$.usage.cost')),
                MAX(json_extract(meta_json,'$.usage.currency'))
         FROM messages
         WHERE role = 'agent' AND json_extract(meta_json,'$.usage.account_id') = ?1 AND ts >= ?2
           AND (?3 IS NULL OR session_id = ?3)",
        params![account, since, session],
        |r| {
            Ok(Sums {
                tokens_in: r.get(0)?,
                tokens_out: r.get(1)?,
                cost: r.get(2)?,
                currency: r.get(3)?,
            })
        },
    )
}

struct Row {
    kind: String,
    window_h: Option<f64>,
    used: Option<f64>,
    rate: Option<f64>,
    load: Option<f64>,
    limit_requests: Option<i64>,
    limit_tokens: Option<i64>,
    remaining: Option<f64>,
    reset_at: Option<i64>,
    updated_at: i64,
}

fn row(c: &Connection, account: i64) -> rusqlite::Result<Option<Row>> {
    c.query_row(
        "SELECT kind, window_h, used, rate, load, limit_requests, limit_tokens, remaining, reset_at, updated_at
         FROM quotas WHERE account_id = ?1",
        [account],
        |r| {
            Ok(Row {
                kind: r.get(0)?,
                window_h: r.get(1)?,
                used: r.get(2)?,
                rate: r.get(3)?,
                load: r.get(4)?,
                limit_requests: r.get(5)?,
                limit_tokens: r.get(6)?,
                remaining: r.get(7)?,
                reset_at: r.get(8)?,
                updated_at: r.get(9)?,
            })
        },
    )
    .optional()
}

fn limit_json(w: Option<LimitWindow>) -> Value {
    w.map_or(
        Value::Null,
        |w| json!({"limit": w.limit, "remaining": w.remaining, "reset_at": w.reset_at}),
    )
}

/// Windows from the columns (after a restart the exact windows are gone): remainder is the share of the tightest window from each limit.
fn windows_from_row(r: &Row) -> RateLimit {
    let mk = |limit: Option<i64>| {
        let (limit, frac) = (u64::try_from(limit?).ok()?, r.remaining?);
        Some(LimitWindow {
            limit,
            remaining: (frac * limit as f64).round() as u64,
            reset_at: r.reset_at,
        })
    };
    RateLimit {
        requests: mk(r.limit_requests),
        tokens: mk(r.limit_tokens),
    }
}

fn quota_json(
    c: &Connection,
    account: i64,
    day_start: i64,
    session: Option<i64>,
    cached: Option<RateLimit>,
) -> rusqlite::Result<Option<Value>> {
    let Some(r) = row(c, account)? else {
        return Ok(None);
    };
    let s = sums(
        c,
        account,
        if session.is_some() { 0 } else { day_start },
        session,
    )?;
    let w = cached.unwrap_or_else(|| windows_from_row(&r));
    let limits = if w.is_empty() {
        Value::Null
    } else {
        json!({"requests_per_min": limit_json(w.requests), "tokens_per_min": limit_json(w.tokens)})
    };
    let eta = match (r.used, r.rate) {
        (Some(u), Some(rate)) => eta_at(u, rate, r.updated_at, r.reset_at),
        _ => None,
    };
    let mut usage = json!({
        "period": if session.is_some() { "session" } else { "day" },
        "tokens_in": s.tokens_in, "tokens_out": s.tokens_out, "cost": s.cost, "currency": s.currency,
    });
    if let Some(id) = session {
        usage["session_id"] = json!(id);
    }
    Ok(Some(json!({
        "account_id": account, "kind": r.kind, "window_h": r.window_h, "used": r.used, "rate": r.rate,
        "reset_at": r.reset_at, "eta_at": eta, "load": r.load, "usage": usage, "limits": limits,
        "updated_at": r.updated_at,
    })))
}

/// Recompute the `quotas` row and write to `quota_samples`; `false` if the account no longer exists.
fn apply(tx: &Transaction<'_>, rec: &UsageRecord, day_start: i64) -> rusqlite::Result<bool> {
    let alive = tx
        .query_row(
            "SELECT 1 FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
            [rec.account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !alive {
        return Ok(false);
    }
    let s = sums(tx, rec.account_id, day_start, None)?;
    let prev = row(tx, rec.account_id)?;
    let new = rec.rate.filter(|r| !r.is_empty());
    // (kind, window_h, used, rate, limit_requests, limit_tokens, remaining, reset_at)
    type Cols = (
        &'static str,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<i64>,
        Option<i64>,
        Option<f64>,
        Option<i64>,
    );
    let cols: Cols = if rec.local {
        ("local", Some(0.0), None, None, None, None, None, None)
    } else if let Some(w) = new.as_ref().and_then(tightest) {
        let rate = new.unwrap_or_default();
        let lim = |x: Option<LimitWindow>| x.and_then(|w| i64::try_from(w.limit).ok());
        (
            "api_key",
            Some(WINDOW_SECS as f64 / 3600.0),
            Some(used_fraction(&w)),
            Some(rate_of(&w, rec.now)),
            lim(rate.requests),
            lim(rate.tokens),
            Some(1.0 - used_fraction(&w)),
            w.reset_at,
        )
    } else if let Some(p) = prev {
        // No new limits, so the previous ones stay: the provider may simply not have sent headers.
        (
            "api_key",
            p.window_h,
            p.used,
            p.rate,
            p.limit_requests,
            p.limit_tokens,
            p.remaining,
            p.reset_at,
        )
    } else {
        ("api_key", None, None, None, None, None, None, None)
    };
    let (kind, window_h, used, rate, lr, lt, remaining, reset_at) = cols;
    let total = s.tokens_in + s.tokens_out;
    tx.execute(
        "INSERT INTO quotas(account_id, kind, window_h, used, rate, usage_tokens, usage_cost, limit_requests, limit_tokens, remaining, reset_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(account_id) DO UPDATE SET kind=?2, window_h=?3, used=?4, rate=?5, usage_tokens=?6, usage_cost=?7,
           limit_requests=?8, limit_tokens=?9, remaining=?10, reset_at=?11, updated_at=?12",
        params![rec.account_id, kind, window_h, used, rate, total, s.cost, lr, lt, remaining, reset_at, rec.now],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO quota_samples(account_id, ts, used, usage_tokens, cost) VALUES (?1,?2,?3,?4,?5)",
        params![rec.account_id, rec.now, used, total, s.cost],
    )?;
    Ok(true)
}

impl QuotaStore for QuotaRepo {
    fn record(&self, rec: UsageRecord) -> PortFuture<'_, Option<Value>> {
        Box::pin(async move {
            let day = rec.now - rec.now.rem_euclid(DAY_SECS);
            let (account, fresh) = (rec.account_id, rec.rate.filter(|r| !r.is_empty()));
            let rec2 = rec.clone();
            let ok = self.db.write(move |tx| apply(tx, &rec2, day)).await.ok()?;
            if !ok {
                return None;
            }
            if let Some(r) = fresh.filter(|_| !rec.local) {
                self.windows().insert(account, r);
            }
            let cached = self.windows().get(&account).copied();
            self.db
                .read(move |c| quota_json(c, account, day, None, cached))
                .await
                .ok()?
        })
    }

    fn quotas(&self, session_id: Option<i64>) -> PortFuture<'_, Option<Value>> {
        Box::pin(async move {
            let day = self.clock.now_unix() - self.clock.now_unix().rem_euclid(DAY_SECS);
            let cache = self.windows().clone();
            self.db
                .read(move |c| {
                    let ids: Vec<i64> = c
                        .prepare("SELECT q.account_id FROM quotas q JOIN accounts a ON a.id = q.account_id WHERE a.deleted_at IS NULL ORDER BY q.account_id")?
                        .query_map([], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    let mut items = Vec::new();
                    for id in ids {
                        if let Some(q) = quota_json(c, id, day, session_id, cache.get(&id).copied())? {
                            // For a session, show only accounts that responded in it.
                            let used_here = session_id.is_none()
                                || q["usage"]["tokens_in"].as_i64().unwrap_or(0) + q["usage"]["tokens_out"].as_i64().unwrap_or(0) > 0;
                            if used_here {
                                items.push(q);
                            }
                        }
                    }
                    Ok(json!({"items": items}))
                })
                .await
                .ok()
        })
    }
}
