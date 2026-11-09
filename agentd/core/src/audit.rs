//! Action log and observer findings (`docs/reference/db.md` 3.7, 3.11): types, store port and a write service with masking.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::{Action, Clock, PortFuture};

/// Permission and log store error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermError {
    /// Object (persona, session, log entry) not found.
    NotFound,
    Unavailable,
}

pub type PermResult<T> = Result<T, PermError>;

/// `actions_log.actor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Agent,
    Cron,
    User,
    Observer,
    Module,
}

impl Actor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Actor::Agent => "agent",
            Actor::Cron => "cron",
            Actor::User => "user",
            Actor::Observer => "observer",
            Actor::Module => "module",
        }
    }
}

/// `actions_log.decision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AskApproved,
    AskDenied,
    Deny,
}

impl Decision {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::AskApproved => "ask-approved",
            Decision::AskDenied => "ask-denied",
            Decision::Deny => "deny",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Decision> {
        match s {
            "allow" => Some(Decision::Allow),
            "ask-approved" => Some(Decision::AskApproved),
            "ask-denied" => Some(Decision::AskDenied),
            "deny" => Some(Decision::Deny),
            _ => None,
        }
    }
}

/// New log entry. `action` is a string: the log also records non-permission actions (`token_issue`, `permission_put`).
#[derive(Debug, Clone)]
pub struct NewAction {
    pub actor: Actor,
    pub session_id: Option<i64>,
    pub persona_id: Option<i64>,
    pub action: String,
    pub target: Option<String>,
    /// Arguments before masking: the write service masks them itself.
    pub args: Option<Value>,
    pub decision: Option<Decision>,
    pub result: Option<String>,
}

/// An `actions_log` row as it lies in the DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRow {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub session_id: Option<i64>,
    pub persona_id: Option<i64>,
    pub action: String,
    pub target: Option<String>,
    pub args_json: Option<String>,
    pub decision: Option<Decision>,
    pub result: Option<String>,
    pub reversible: bool,
    pub undo_json: Option<String>,
}

/// Observer finding (`observer_findings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// `injection|phishing|dangerous_cmd|module|permission_loosen`.
    pub kind: &'static str,
    /// `low|med|high`.
    pub severity: &'static str,
    /// `actions_log|permissions|…`.
    pub ref_table: Option<&'static str>,
    pub ref_id: Option<String>,
    /// `blocked|asked|allowed`.
    pub verdict: &'static str,
    pub detail: Option<String>,
}

/// Log page filter (`GET /v1/actions`); `before` is a cursor: only entries with `id` below it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionQuery {
    pub session_id: Option<i64>,
    pub actor: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub before: Option<i64>,
    pub limit: usize,
}

/// Log and findings store (implemented by `agentd-db`).
pub trait AuditStore: Send + Sync + 'static {
    /// Writes a row; `args_json` is already masked. Returns the `id`.
    fn insert_action(
        &self,
        ts: i64,
        entry: NewAction,
        args_json: Option<String>,
    ) -> PortFuture<'_, PermResult<i64>>;
    /// Log page, newest first (`ORDER BY id DESC`), at most `q.limit` entries.
    fn list_actions(&self, q: ActionQuery) -> PortFuture<'_, PermResult<Vec<ActionRow>>>;
    fn get_action(&self, id: i64) -> PortFuture<'_, PermResult<Option<ActionRow>>>;
    /// Decision on an entry without a decision (`decision IS NULL`); `false` — already decided or no such entry (first-wins).
    /// `args_json` replaces the column if given.
    fn decide_action(
        &self,
        id: i64,
        decision: Decision,
        result: Option<String>,
        args_json: Option<String>,
    ) -> PortFuture<'_, PermResult<bool>>;
    /// Entries without a decision (open permission requests); `session_id` limits to a session.
    fn pending_actions(
        &self,
        session_id: Option<i64>,
    ) -> PortFuture<'_, PermResult<Vec<ActionRow>>>;
    /// Action execution outcome (`result`): replaces the decision-stage outcome (`allowed`).
    fn set_result(&self, id: i64, result: String) -> PortFuture<'_, PermResult<()>>;
    fn insert_finding(&self, ts: i64, f: Finding) -> PortFuture<'_, PermResult<i64>>;
}

const SECRET_KEYS: [&str; 6] = [
    "token",
    "key",
    "secret",
    "password",
    "authorization",
    "passwd",
];

/// Mask in place of a value.
pub const MASK: &str = "***";

fn secret_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|s| k.contains(s))
}

/// The value looks like a token or key: `sk-…`, `Bearer …`, a long string of token characters, `agt_`/`aos_` tokens.
fn looks_like_token(s: &str) -> bool {
    let t = s.trim();
    if t.get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("bearer "))
    {
        return true;
    }
    if ["sk-", "ghp_", "gho_", "xox", "AKIA", "aos_", "agt_"]
        .iter()
        .any(|p| t.starts_with(p))
        && t.len() >= 12
    {
        return true;
    }
    t.len() >= 32
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '='))
        && t.chars().any(|c| c.is_ascii_digit())
}

/// Copy of the arguments with secrets masked: values under the keys `token`, `key`, `secret`, `password`,
/// `authorization` (by occurrence in the name, case-insensitive) and strings matching the token pattern.
#[must_use]
pub fn mask_args(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, val) in o {
                let masked = if secret_key(k) && !val.is_null() {
                    Value::String(MASK.into())
                } else {
                    mask_args(val)
                };
                out.insert(k.clone(), masked);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(mask_args).collect()),
        Value::String(s) if looks_like_token(s) => Value::String(MASK.into()),
        other => other.clone(),
    }
}

/// String with masked token words: `Bearer …`, `sk-…` and other values matching the token pattern.
#[must_use]
pub fn mask_text(s: &str) -> String {
    let mut out = Vec::new();
    let mut after_bearer = false;
    for w in s.split(' ') {
        let hide = after_bearer || looks_like_token(w);
        after_bearer = w.eq_ignore_ascii_case("bearer");
        out.push(if hide { MASK } else { w });
    }
    out.join(" ")
}

/// Log write service: masks arguments, sets the time, does not allow marking the irreversible as reversible.
#[derive(Clone)]
pub struct Audit {
    store: Arc<dyn AuditStore>,
    clock: Arc<dyn Clock>,
    events: Option<crate::Events>,
}

impl Audit {
    #[must_use]
    pub fn new(store: Arc<dyn AuditStore>, clock: Arc<dyn Clock>) -> Audit {
        Audit {
            store,
            clock,
            events: None,
        }
    }

    /// Observer findings go to the event feed (`ai.watch`).
    #[must_use]
    pub fn with_events(mut self, events: crate::Events) -> Audit {
        self.events = Some(events);
        self
    }

    pub async fn record(&self, mut entry: NewAction) -> PermResult<i64> {
        entry.target = entry.target.as_deref().map(mask_text);
        let args = entry.args.as_ref().map(|a| mask_args(a).to_string());
        self.store
            .insert_action(self.clock.now_unix(), entry, args)
            .await
    }

    pub async fn finding(&self, f: Finding) -> PermResult<i64> {
        let (kind, verdict) = (f.kind, f.verdict);
        let id = self.store.insert_finding(self.clock.now_unix(), f).await?;
        if let Some(events) = &self.events {
            let level = match verdict {
                "blocked" => crate::EventLevel::Error,
                "asked" => crate::EventLevel::Warn,
                _ => crate::EventLevel::Info,
            };
            let text = format!("Observer: {kind}, {verdict}");
            events
                .emit(crate::NewEvent::new("ai", "watch", level, text))
                .await;
        }
        Ok(id)
    }

    #[must_use]
    pub fn store(&self) -> &Arc<dyn AuditStore> {
        &self.store
    }

    /// The log also records actions outside the permission set; for the set — the action string.
    #[must_use]
    pub fn action_name(a: Action) -> String {
        a.as_str().to_string()
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
