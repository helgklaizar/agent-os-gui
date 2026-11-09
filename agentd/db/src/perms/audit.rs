//! `AuditStore` for `PermRepo`: action log and observer findings.

use agentd_core::{
    ActionQuery, ActionRow, AuditStore, Decision, Finding, NewAction, PermResult, PortFuture,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{PermRepo, unavailable};

pub(super) const ACTION_COLS: &str = "id, ts, actor, session_id, persona_id, action, target, args_json, decision, result, reversible, undo_json";

pub(crate) fn action_row(r: &Row<'_>) -> rusqlite::Result<ActionRow> {
    Ok(ActionRow {
        id: r.get(0)?,
        ts: r.get(1)?,
        actor: r.get(2)?,
        session_id: r.get(3)?,
        persona_id: r.get(4)?,
        action: r.get(5)?,
        target: r.get(6)?,
        args_json: r.get(7)?,
        decision: r
            .get::<_, Option<String>>(8)?
            .and_then(|d| Decision::parse(&d)),
        result: r.get(9)?,
        reversible: r.get::<_, i64>(10)? != 0,
        undo_json: r.get(11)?,
    })
}

impl AuditStore for PermRepo {
    fn insert_action(
        &self,
        ts: i64,
        e: NewAction,
        args_json: Option<String>,
    ) -> PortFuture<'_, PermResult<i64>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    // Rollback from 0.1 removed (Q10): `reversible = 0` and `undo_json = NULL` for all records, `send` and `pay` first of all.
                    tx.execute(
                        "INSERT INTO actions_log(ts, actor, session_id, persona_id, action, target, args_json, decision, result, reversible, undo_json)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,NULL)",
                        params![
                            ts,
                            e.actor.as_str(),
                            e.session_id,
                            e.persona_id,
                            e.action,
                            e.target,
                            args_json,
                            e.decision.map(Decision::as_str),
                            e.result
                        ],
                    )?;
                    Ok(tx.last_insert_rowid())
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn list_actions(&self, q: ActionQuery) -> PortFuture<'_, PermResult<Vec<ActionRow>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    let mut stmt = c.prepare(&format!(
                        "SELECT {ACTION_COLS} FROM actions_log
                         WHERE (?1 IS NULL OR session_id = ?1) AND (?2 IS NULL OR actor = ?2)
                           AND (?3 IS NULL OR ts >= ?3) AND (?4 IS NULL OR ts <= ?4)
                           AND (?5 IS NULL OR id < ?5)
                         ORDER BY id DESC LIMIT ?6"
                    ))?;
                    let limit = i64::try_from(q.limit).unwrap_or(i64::MAX);
                    stmt.query_map(
                        params![q.session_id, q.actor, q.from, q.to, q.before, limit],
                        action_row,
                    )?
                    .collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn get_action(&self, id: i64) -> PortFuture<'_, PermResult<Option<ActionRow>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    c.query_row(
                        &format!("SELECT {ACTION_COLS} FROM actions_log WHERE id = ?1"),
                        [id],
                        action_row,
                    )
                    .optional()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn decide_action(
        &self,
        id: i64,
        decision: Decision,
        result: Option<String>,
        args_json: Option<String>,
    ) -> PortFuture<'_, PermResult<bool>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    // First-wins: the decision is written only to a record without a decision.
                    let n = tx.execute(
                        "UPDATE actions_log SET decision = ?2, result = ?3, args_json = COALESCE(?4, args_json)
                         WHERE id = ?1 AND decision IS NULL",
                        params![id, decision.as_str(), result, args_json],
                    )?;
                    Ok(n == 1)
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn pending_actions(
        &self,
        session_id: Option<i64>,
    ) -> PortFuture<'_, PermResult<Vec<ActionRow>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    let mut stmt = c.prepare(&format!(
                        "SELECT {ACTION_COLS} FROM actions_log WHERE decision IS NULL AND (?1 IS NULL OR session_id = ?1) ORDER BY id"
                    ))?;
                    stmt.query_map([session_id], action_row)?.collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn set_result(&self, id: i64, result: String) -> PortFuture<'_, PermResult<()>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "UPDATE actions_log SET result = ?2 WHERE id = ?1",
                        params![id, result],
                    )
                    .map(drop)
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn insert_finding(&self, ts: i64, f: Finding) -> PortFuture<'_, PermResult<i64>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict, detail)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        params![ts, f.kind, f.severity, f.ref_table, f.ref_id, f.verdict, f.detail],
                    )?;
                    Ok(tx.last_insert_rowid())
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }
}

/// Open permission requests of a session for `snapshot.open_requests`: records without a decision that have the request fields saved.
pub(crate) fn open_requests(
    c: &Connection,
    session_id: i64,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {ACTION_COLS} FROM actions_log WHERE session_id = ?1 AND decision IS NULL ORDER BY id"
    ))?;
    let rows = stmt.query_map([session_id], action_row)?;
    let mut out = Vec::new();
    for r in rows {
        if let Some(req) = agentd_core::PermissionRequest::from_row(&r?) {
            out.push(req.to_json());
        }
    }
    Ok(out)
}
