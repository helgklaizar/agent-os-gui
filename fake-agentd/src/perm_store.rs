//! Permissions, action log and watcher findings: DB rows and the tightening rule.
//!
//! The effective mode is the strictest of the explicit rules `global -> persona -> session` (`deny > ask > allow`);
//! without explicit rules the default mode applies: `ask` for `send`, `delete`, `pay`, `config_write`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::mask::mask_value;
use crate::tree;

pub const ACTIONS: [&str; 8] = [
    "read",
    "send",
    "delete",
    "pay",
    "shell",
    "browse",
    "memory_write",
    "config_write",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Allow,
    Ask,
    Deny,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "allow" => Some(Mode::Allow),
            "ask" => Some(Mode::Ask),
            "deny" => Some(Mode::Deny),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Allow => "allow",
            Mode::Ask => "ask",
            Mode::Deny => "deny",
        }
    }
}

pub fn default_mode(action: &str) -> Mode {
    match action {
        "send" | "delete" | "pay" | "config_write" => Mode::Ask,
        _ => Mode::Allow,
    }
}

/// Actions blocked when the watcher is unavailable (fail-closed).
pub fn dangerous(action: &str) -> bool {
    matches!(action, "delete" | "pay" | "shell" | "send" | "config_write")
}

fn explicit(
    conn: &Connection,
    scope: &str,
    id: i64,
    action: &str,
) -> rusqlite::Result<Option<Mode>> {
    let m: Option<String> = conn
        .query_row(
            "SELECT mode FROM permissions WHERE scope = ?1 AND scope_id = ?2 AND action = ?3",
            params![scope, id, action],
            |r| r.get(0),
        )
        .optional()?;
    Ok(m.as_deref().and_then(Mode::parse))
}

/// The strictest explicit rule of the higher levels for `(scope, scope_id)`.
fn above(conn: &Connection, scope: &str, id: i64, action: &str) -> rusqlite::Result<Option<Mode>> {
    let mut found = Vec::new();
    if scope != "global" {
        found.push(explicit(conn, "global", 0, action)?);
    }
    if scope == "session"
        && let Some(p) = tree::effective_persona_id(conn, id)?
    {
        found.push(explicit(conn, "persona", p, action)?);
    }
    Ok(found.into_iter().flatten().max())
}

pub fn effective(conn: &Connection, session: Option<i64>, action: &str) -> rusqlite::Result<Mode> {
    let mut found = vec![explicit(conn, "global", 0, action)?];
    if let Some(s) = session {
        found.push(explicit(conn, "session", s, action)?);
        if let Some(p) = tree::effective_persona_id(conn, s)? {
            found.push(explicit(conn, "persona", p, action)?);
        }
    }
    Ok(found
        .into_iter()
        .flatten()
        .max()
        .unwrap_or_else(|| default_mode(action)))
}

pub enum PutError {
    /// A lower level tries to loosen a higher-level rule.
    Loosen,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for PutError {
    fn from(e: rusqlite::Error) -> Self {
        PutError::Db(e)
    }
}

/// Writes a rule; loosening a higher-level rule is rejected, a `permission_loosen` finding is written here too.
pub fn put_rule(
    conn: &Connection,
    now: i64,
    (scope, id, action): (&str, i64, &str),
    mode: Mode,
    who: &str,
    keep_own: bool,
) -> Result<(), PutError> {
    // "Always" does not loosen even its own level: a `deny` rule stays until an admin changes it explicitly (`PUT`)
    let own = if keep_own {
        explicit(conn, scope, id, action)?
    } else {
        None
    };
    if above(conn, scope, id, action)?
        .into_iter()
        .chain(own)
        .any(|a| mode < a)
    {
        let detail = format!(
            "{who}: {scope}/{id}/{action} = {} loosens the higher-level rule",
            mode.as_str()
        );
        finding(
            conn,
            now,
            "permission_loosen",
            "med",
            Some("permissions"),
            Some(&format!("{scope}/{id}/{action}")),
            "blocked",
            Some(&detail),
        )?;
        return Err(PutError::Loosen);
    }
    conn.execute(
        "INSERT INTO permissions(scope, scope_id, action, mode) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(scope, scope_id, action) DO UPDATE SET mode = excluded.mode",
        params![scope, id, action, mode.as_str()],
    )?;
    Ok(())
}

pub fn permissions_json(
    conn: &Connection,
    scope: Option<&str>,
    id: Option<i64>,
) -> rusqlite::Result<Vec<Value>> {
    let mut st = conn.prepare(
        "SELECT scope, scope_id, action, mode FROM permissions \
         WHERE (?1 IS NULL OR scope = ?1) AND (?2 IS NULL OR scope_id = ?2) ORDER BY scope, scope_id, action",
    )?;
    let rows = st.query_map(params![scope, id], |r| {
        Ok(
            json!({ "scope": r.get::<_, String>(0)?, "scope_id": r.get::<_, i64>(1)?,
                   "action": r.get::<_, String>(2)?, "mode": r.get::<_, String>(3)? }),
        )
    })?;
    rows.collect()
}

/// Log entry; `args` are masked, `reversible = 0` for irreversible actions (sending, payment).
#[allow(clippy::too_many_arguments)]
pub fn log_action(
    conn: &Connection,
    now: i64,
    actor: &str,
    (session, persona): (Option<i64>, Option<i64>),
    (action, target): (&str, Option<&str>),
    args: Option<&Value>,
    (decision, result): (Option<&str>, Option<&str>),
) -> rusqlite::Result<i64> {
    let args = args.map(|a| mask_value(a).to_string());
    let target = target.map(crate::mask::mask_text);
    let reversible = i64::from(!matches!(action, "send" | "pay"));
    conn.execute(
        "INSERT INTO actions_log(ts, actor, session_id, persona_id, action, target, args_json, decision, result, reversible) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![now, actor, session, persona, action, target, args, decision, result, reversible],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_decision(
    conn: &Connection,
    id: i64,
    decision: &str,
    result: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE actions_log SET decision = ?1, result = ?2 WHERE id = ?3",
        params![decision, result, id],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn finding(
    conn: &Connection,
    now: i64,
    kind: &str,
    severity: &str,
    ref_table: Option<&str>,
    ref_id: Option<&str>,
    verdict: &str,
    detail: Option<&str>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![now, kind, severity, ref_table, ref_id, verdict, detail],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_finding_verdict(conn: &Connection, id: i64, verdict: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE observer_findings SET verdict = ?1 WHERE id = ?2",
        params![verdict, id],
    )?;
    Ok(())
}
