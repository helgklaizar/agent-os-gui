//! Reply context: persona, model account, `AGENT.md`, history tail.

use std::path::Path;

use agentd_core::{AccountRow, ChatContext, HistoryItem};
use rusqlite::{Connection, OptionalExtension, params};

const AGENT_MD_MAX: u64 = 64 * 1024;

fn read_agent_md(root: &Path) -> Option<String> {
    let p = root.join("AGENT.md");
    let meta = std::fs::metadata(&p).ok()?;
    if !meta.is_file() || meta.len() > AGENT_MD_MAX {
        return None;
    }
    std::fs::read_to_string(p)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn context(
    c: &Connection,
    root: &Path,
    session_id: i64,
) -> rusqlite::Result<Option<ChatContext>> {
    // The effective persona and name are computed by the same query as `Session.effective_persona_id`.
    let Some(session) = crate::tree::session_json(c, session_id)? else {
        return Ok(None);
    };
    let cfg_model = session["cfg"]["model"].as_i64();
    let persona_id = session["effective_persona_id"].as_i64();
    let mut ctx = ChatContext {
        session_name: session["name"].as_str().unwrap_or_default().to_string(),
        cfg_model,
        ctx_parent: session["cfg"]["ctx"] == "parent",
        agent_md: read_agent_md(root),
        ..ChatContext::default()
    };
    ctx.persona_id = persona_id;
    if let Some(pid) = persona_id {
        let row = c
            .query_row(
                "SELECT name, style, lang, signature, model_acc FROM personas WHERE id = ?1 AND deleted_at IS NULL",
                [pid],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<i64>>(4)?)),
            )
            .optional()?;
        if let Some((name, style, lang, signature, acc)) = row {
            ctx.persona_name = Some(name);
            ctx.style = style;
            ctx.lang = lang;
            ctx.signature = signature;
            ctx.persona_model_acc = acc;
        }
    }
    ctx.facts = crate::memory::context_facts(c, session_id)?;
    Ok(Some(ctx))
}

pub(crate) fn account(c: &Connection, id: i64) -> rusqlite::Result<Option<AccountRow>> {
    c.query_row(
        "SELECT id, type, label, secret_ref FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        |r| {
            Ok(AccountRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                label: r.get(2)?,
                secret_ref: r.get(3)?,
            })
        },
    )
    .optional()
}

/// User and agent turns and tool results (for the model they are user turns) with ready text (`done`, `stopped`) up to `before_id`, oldest to newest; collected from
/// the end while it fits in `max_bytes` (at least one turn is always taken).
fn tail(
    c: &Connection,
    session_id: i64,
    before_id: i64,
    max_bytes: usize,
) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut stmt = c.prepare(
        "SELECT role, body_md FROM messages WHERE session_id = ?1 AND id < ?2 AND role IN ('user','agent','tool')
           AND status IN ('done','stopped') AND body_md <> '' ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![session_id, before_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    let mut used = 0usize;
    for row in rows {
        let (role, text) = row?;
        if !out.is_empty() && used + text.len() > max_bytes {
            break;
        }
        used += text.len();
        out.push(HistoryItem {
            agent: role == "agent",
            text,
        });
    }
    out.reverse();
    Ok(out)
}

pub(crate) fn history(
    c: &Connection,
    session_id: i64,
    before_id: i64,
    with_parent: bool,
    max_bytes: usize,
) -> rusqlite::Result<Vec<HistoryItem>> {
    let own = tail(c, session_id, before_id, max_bytes)?;
    let used: usize = own.iter().map(|h| h.text.len()).sum();
    let parent: Option<i64> = c
        .query_row(
            "SELECT parent_id FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut all = Vec::new();
    if let (true, Some(p)) = (with_parent, parent) {
        all = tail(c, p, i64::MAX, max_bytes.saturating_sub(used))?;
    }
    all.extend(own);
    Ok(all)
}
