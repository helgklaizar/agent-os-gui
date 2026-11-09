//! Rules, action log and observer findings over the `permissions`, `actions_log`, `observer_findings` tables.
//! Writes go through the single writer; each operation is one transaction.

use std::sync::Arc;

use agentd_core::{
    Action, Level, Mode, PermError, PermResult, PortFuture, Rule, RuleSet, RuleStore,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::{Db, DbError};

mod audit;

pub(crate) use audit::open_requests;

pub struct PermRepo {
    pub(super) db: Arc<Db>,
}

impl PermRepo {
    #[must_use]
    pub fn new(db: Arc<Db>) -> PermRepo {
        PermRepo { db }
    }
}

pub(super) fn unavailable(e: &DbError) -> PermError {
    tracing::warn!(scope = "perms", event = "db_unavailable", error = %e, "permissions DB unavailable");
    PermError::Unavailable
}

/// Rows outside the `CHECK` set do not occur in the DB; in case of a manual edit such a row is skipped.
fn rule_of(r: &Row<'_>) -> rusqlite::Result<Option<Rule>> {
    let (scope, scope_id, action, mode): (String, i64, String, String) =
        (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    Ok(
        match (
            Level::parse(&scope),
            Action::parse(&action),
            Mode::parse(&mode),
        ) {
            (Some(level), Some(action), Some(mode)) => Some(Rule {
                level,
                scope_id,
                action,
                mode,
            }),
            _ => None,
        },
    )
}

fn rule_set(
    c: &Connection,
    persona: Option<i64>,
    session: Option<i64>,
    action: Action,
) -> rusqlite::Result<RuleSet> {
    let mut stmt = c.prepare(
        "SELECT scope, scope_id, action, mode FROM permissions WHERE action = ?1 AND
         ((scope = 'global' AND scope_id = 0) OR (scope = 'persona' AND scope_id = ?2) OR (scope = 'session' AND scope_id = ?3))",
    )?;
    let rows = stmt.query_map(
        params![
            action.as_str(),
            persona.unwrap_or(-1),
            session.unwrap_or(-1)
        ],
        rule_of,
    )?;
    let mut set = RuleSet::default();
    for r in rows {
        if let Some(rule) = r? {
            match rule.level {
                Level::Global => set.global = Some(rule.mode),
                Level::Persona => set.persona = Some(rule.mode),
                Level::Session => set.session = Some(rule.mode),
            }
        }
    }
    Ok(set)
}

impl RuleStore for PermRepo {
    fn rule_set(
        &self,
        persona_id: Option<i64>,
        session_id: Option<i64>,
        action: Action,
    ) -> PortFuture<'_, PermResult<RuleSet>> {
        Box::pin(async move {
            self.db
                .read(move |c| rule_set(c, persona_id, session_id, action))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn list_rules(
        &self,
        level: Option<Level>,
        scope_id: Option<i64>,
    ) -> PortFuture<'_, PermResult<Vec<Rule>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    let mut stmt = c.prepare(
                        "SELECT scope, scope_id, action, mode FROM permissions
                         WHERE (?1 IS NULL OR scope = ?1) AND (?2 IS NULL OR scope_id = ?2)
                         ORDER BY scope, scope_id, action",
                    )?;
                    let rows =
                        stmt.query_map(params![level.map(Level::as_str), scope_id], rule_of)?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.extend(r?);
                    }
                    Ok(out)
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn put_rule(&self, rule: Rule) -> PortFuture<'_, PermResult<()>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "INSERT INTO permissions(scope, scope_id, action, mode) VALUES (?1,?2,?3,?4)
                         ON CONFLICT(scope, scope_id, action) DO UPDATE SET mode = excluded.mode",
                        params![
                            rule.level.as_str(),
                            rule.scope_id,
                            rule.action.as_str(),
                            rule.mode.as_str()
                        ],
                    )
                    .map(drop)
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn session_persona(&self, session_id: i64) -> PortFuture<'_, PermResult<Option<i64>>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    Ok(crate::tree::session_json(c, session_id)?
                        .and_then(|s| s["effective_persona_id"].as_i64()))
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn scope_exists(&self, level: Level, scope_id: i64) -> PortFuture<'_, PermResult<bool>> {
        Box::pin(async move {
            let sql = match level {
                Level::Global => return Ok(scope_id == 0),
                Level::Persona => "SELECT 1 FROM personas WHERE id = ?1 AND deleted_at IS NULL",
                Level::Session => "SELECT 1 FROM sessions WHERE id = ?1",
            };
            self.db
                .read(move |c| {
                    Ok(c.query_row(sql, [scope_id], |_| Ok(()))
                        .optional()?
                        .is_some())
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn seed_defaults(&self) -> PortFuture<'_, PermResult<usize>> {
        Box::pin(async move {
            self.db
                .write(|tx| {
                    let mut n = 0;
                    for a in Action::ALL {
                        if let Some(mode) = a.default_mode() {
                            n += tx.execute(
                                "INSERT OR IGNORE INTO permissions(scope, scope_id, action, mode) VALUES ('global',0,?1,?2)",
                                params![a.as_str(), mode.as_str()],
                            )?;
                        }
                    }
                    Ok(n)
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }
}
