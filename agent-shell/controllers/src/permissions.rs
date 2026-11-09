//! Permissions: the effective mode follows the rule "a lower level can only tighten" (ADR-0005).

/// Permission mode; the order `Allow < Ask < Deny` is strictness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Allow,
    Ask,
    Deny,
}

/// Level of a rule that can be created with the "Always" answer (there is no session level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleScope {
    Persona,
    Global,
}

/// Decision on a permission request (`PermissionDecision.decision`, docs/reference/api.md §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermDecision {
    Allow,
    Deny,
    Always,
}

/// Rule for `POST /permission-requests/{id}/decision` with `always`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleSpec {
    pub scope: RuleScope,
    pub scope_id: i64,
    pub mode: Mode,
}

/// The strictest of the given levels; `None` means "as the persona" or no rule. No values at all gives `Allow`
/// (neutral: the action default is added by [`effective_for_action`]).
pub fn effective_mode(global: Option<Mode>, persona: Option<Mode>, session: Option<Mode>) -> Mode {
    [global, persona, session]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(Mode::Allow)
}

/// Default mode: `send`, `delete`, `pay`, `config_write` are `ask`, everything else is `allow`.
pub fn default_mode(action: &str) -> Mode {
    match action {
        "send" | "delete" | "pay" | "config_write" => Mode::Ask,
        _ => Mode::Allow,
    }
}

/// Effective action mode: with no rules at all, the default mode.
pub fn effective_for_action(
    action: &str,
    global: Option<Mode>,
    persona: Option<Mode>,
    session: Option<Mode>,
) -> Mode {
    if global.is_none() && persona.is_none() && session.is_none() {
        return default_mode(action);
    }
    effective_mode(global, persona, session)
}

fn not_weaker_than(floor: Mode) -> Vec<Mode> {
    [Mode::Allow, Mode::Ask, Mode::Deny]
        .into_iter()
        .filter(|m| *m >= floor)
        .collect()
}

/// Values the view offers at session level: none that weaken a higher-level rule.
pub fn session_options(global: Option<Mode>, persona: Option<Mode>) -> Vec<Mode> {
    not_weaker_than(effective_mode(global, persona, None))
}

/// Persona rule values for "Always": not weaker than the global one.
pub fn persona_rule_options(global: Option<Mode>) -> Vec<Mode> {
    not_weaker_than(global.unwrap_or(Mode::Allow))
}

/// A `GET /v1/permissions` row (docs/reference/api.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRow {
    pub scope: String,
    pub scope_id: i64,
    pub action: String,
    pub mode: Mode,
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

/// Permission matrix as the core returns it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermTable {
    pub rows: Vec<PermissionRow>,
}

impl PermTable {
    /// Parse the `GET /permissions` array; unknown rows are skipped.
    pub fn from_value(v: &serde_json::Value) -> PermTable {
        let text = |r: &serde_json::Value, k: &str| r.get(k)?.as_str().map(str::to_string);
        let row = |r: &serde_json::Value| {
            let scope = text(r, "scope")
                .filter(|s| ["global", "persona", "session"].contains(&s.as_str()))?;
            Some(PermissionRow {
                scope,
                scope_id: r
                    .get("scope_id")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                action: text(r, "action")?,
                mode: Mode::parse(&text(r, "mode")?)?,
            })
        };
        PermTable {
            rows: v
                .as_array()
                .map(|a| a.iter().filter_map(row).collect())
                .unwrap_or_default(),
        }
    }

    fn level(&self, scope: &str, id: i64, action: &str) -> Option<Mode> {
        self.rows
            .iter()
            .find(|r| r.scope == scope && r.scope_id == id && r.action == action)
            .map(|r| r.mode)
    }

    pub fn global(&self, action: &str) -> Option<Mode> {
        self.level("global", 0, action)
    }

    pub fn persona(&self, persona_id: Option<i64>, action: &str) -> Option<Mode> {
        self.level("persona", persona_id?, action)
    }

    pub fn session(&self, session_id: Option<i64>, action: &str) -> Option<Mode> {
        self.level("session", session_id?, action)
    }

    /// Write a session rule into the local copy of the matrix (after a successful `PUT`).
    pub fn set_session(&mut self, session_id: i64, action: &str, mode: Mode) {
        match self
            .rows
            .iter_mut()
            .find(|r| r.scope == "session" && r.scope_id == session_id && r.action == action)
        {
            Some(row) => row.mode = mode,
            None => self.rows.push(PermissionRow {
                scope: "session".to_string(),
                scope_id: session_id,
                action: action.to_string(),
                mode,
            }),
        }
    }

    /// Effective mode of `action` for session `session_id` with persona `persona_id`.
    pub fn effective(
        &self,
        action: &str,
        persona_id: Option<i64>,
        session_id: Option<i64>,
    ) -> Mode {
        effective_for_action(
            action,
            self.global(action),
            self.persona(persona_id, action),
            self.session(session_id, action),
        )
    }
}
