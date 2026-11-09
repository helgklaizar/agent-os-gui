//! Rule store (`permissions`) and the effective permissions engine.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::{
    Action, Audit, Finding, Level, Mode, PermError, PermResult, PortFuture, Rule, RuleSet,
    loosening_levels, would_loosen,
};

/// Rule store (implemented by `agentd-db`).
pub trait RuleStore: Send + Sync + 'static {
    /// Raw rows of three levels for an action.
    fn rule_set(
        &self,
        persona_id: Option<i64>,
        session_id: Option<i64>,
        action: Action,
    ) -> PortFuture<'_, PermResult<RuleSet>>;
    /// All rows, by level and `scope_id` if given.
    fn list_rules(
        &self,
        level: Option<Level>,
        scope_id: Option<i64>,
    ) -> PortFuture<'_, PermResult<Vec<Rule>>>;
    fn put_rule(&self, rule: Rule) -> PortFuture<'_, PermResult<()>>;
    /// Effective persona of the session (inherited from folder and parent); `None` — no persona or no session.
    fn session_persona(&self, session_id: i64) -> PortFuture<'_, PermResult<Option<i64>>>;
    /// Whether a persona or session exists (for `global` — always yes).
    fn scope_exists(&self, level: Level, scope_id: i64) -> PortFuture<'_, PermResult<bool>>;
    /// First-run defaults: global rows where none exist yet. How many were written.
    fn seed_defaults(&self) -> PortFuture<'_, PermResult<usize>>;
}

/// Rule write refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutError {
    /// No such persona or session.
    NotFound,
    /// The rule would loosen a higher level (`403`).
    Loosen,
    Unavailable,
}

impl From<PermError> for PutError {
    fn from(e: PermError) -> PutError {
        match e {
            PermError::NotFound => PutError::NotFound,
            PermError::Unavailable => PutError::Unavailable,
        }
    }
}

/// Permissions engine: reading rules, effective mode, writing with a "tighten only" check.
#[derive(Clone)]
pub struct PermEngine {
    store: Arc<dyn RuleStore>,
    audit: Audit,
    reported: Arc<Mutex<HashSet<(Level, i64, Action)>>>,
    /// Rule set cache: the key is persona, session, action. A write through the engine resets it entirely; an edit bypassing
    /// the engine (manual, cascading delete) becomes visible no later than `ttl`.
    cache: Arc<Mutex<HashMap<CacheKey, (Instant, RuleSet)>>>,
    ttl: Duration,
}

type CacheKey = (Option<i64>, Option<i64>, Action);

/// Default lifetime of a rule cache entry.
pub const RULE_CACHE_TTL: Duration = Duration::from_secs(5);

impl PermEngine {
    #[must_use]
    pub fn new(store: Arc<dyn RuleStore>, audit: Audit) -> PermEngine {
        PermEngine {
            store,
            audit,
            reported: Arc::new(Mutex::new(HashSet::new())),
            cache: Arc::new(Mutex::new(HashMap::new())),
            ttl: RULE_CACHE_TTL,
        }
    }

    /// Lifetime of a rule cache entry.
    #[must_use]
    pub fn with_cache_ttl(mut self, ttl: Duration) -> PermEngine {
        self.ttl = ttl;
        self
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, (Instant, RuleSet)>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Rule set: from the cache while the entry is fresh, otherwise from the store.
    async fn rule_set(
        &self,
        persona_id: Option<i64>,
        session_id: Option<i64>,
        action: Action,
    ) -> PermResult<RuleSet> {
        let key = (persona_id, session_id, action);
        if let Some((at, set)) = self.cache().get(&key).copied()
            && at.elapsed() < self.ttl
        {
            return Ok(set);
        }
        let set = self.store.rule_set(persona_id, session_id, action).await?;
        self.cache().insert(key, (Instant::now(), set));
        Ok(set)
    }

    #[must_use]
    pub fn store(&self) -> &Arc<dyn RuleStore> {
        &self.store
    }

    /// Effective action mode for a persona and session; `None` — no rules. A loosening row in the table (manual
    /// DB edit) has no effect and produces a `permission_loosen` finding (once per row per run).
    pub async fn effective(
        &self,
        persona_id: Option<i64>,
        session_id: Option<i64>,
        action: Action,
    ) -> PermResult<Option<Mode>> {
        let set = self.rule_set(persona_id, session_id, action).await?;
        for level in loosening_levels(&set) {
            let id = match level {
                Level::Persona => persona_id.unwrap_or(0),
                _ => session_id.unwrap_or(0),
            };
            let first = self
                .reported
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((level, id, action));
            if first {
                self.loosen_finding(level, id, action, "table").await;
            }
        }
        Ok(crate::effective_mode(set.global, set.persona, set.session))
    }

    async fn loosen_finding(&self, level: Level, scope_id: i64, action: Action, who: &str) {
        let detail = json!({"level": level.as_str(), "scope_id": scope_id,
                            "action": action.as_str(), "by": who})
        .to_string();
        let f = Finding {
            kind: "permission_loosen",
            severity: "med",
            ref_table: Some("permissions"),
            ref_id: Some(format!("{}/{scope_id}/{}", level.as_str(), action.as_str())),
            verdict: "blocked",
            detail: Some(detail),
        };
        if self.audit.finding(f).await.is_err() {
            tracing::warn!(
                scope = "perms",
                event = "finding_lost",
                "finding not recorded"
            );
        }
    }

    /// Writes a rule. Loosening a higher level — `Loosen` and a finding; `by` — who asked (token label).
    pub async fn put(&self, rule: Rule, by: &str) -> Result<Rule, PutError> {
        if !self.store.scope_exists(rule.level, rule.scope_id).await? {
            return Err(PutError::NotFound);
        }
        let (persona, session) = match rule.level {
            Level::Global => (None, None),
            Level::Persona => (Some(rule.scope_id), None),
            Level::Session => (
                self.store.session_persona(rule.scope_id).await?,
                Some(rule.scope_id),
            ),
        };
        let set = self.store.rule_set(persona, session, rule.action).await?;
        if would_loosen(rule.level, rule.mode, &set) {
            self.loosen_finding(rule.level, rule.scope_id, rule.action, by)
                .await;
            return Err(PutError::Loosen);
        }
        let saved = self.store.put_rule(rule).await;
        // The cache is also reset on a write failure: the outcome is unknown.
        self.cache().clear();
        saved?;
        Ok(rule)
    }
}
