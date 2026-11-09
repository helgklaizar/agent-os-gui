//! Permissions: actions, modes, levels and computing the effective mode. No I/O.

use std::fmt;

use serde_json::{Value, json};

/// Actions that go through permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Read,
    Send,
    Delete,
    Pay,
    Shell,
    Browse,
    MemoryWrite,
    ConfigWrite,
}

impl Action {
    pub const ALL: [Action; 8] = [
        Action::Read,
        Action::Send,
        Action::Delete,
        Action::Pay,
        Action::Shell,
        Action::Browse,
        Action::MemoryWrite,
        Action::ConfigWrite,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Send => "send",
            Action::Delete => "delete",
            Action::Pay => "pay",
            Action::Shell => "shell",
            Action::Browse => "browse",
            Action::MemoryWrite => "memory_write",
            Action::ConfigWrite => "config_write",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// Global default rule: `ask` for `send`, `delete`, `pay`, `config_write`. For the other actions
    /// no value is set: there is no row, the action is not limited by permissions.
    #[must_use]
    pub fn default_mode(self) -> Option<Mode> {
        match self {
            Action::Send | Action::Delete | Action::Pay | Action::ConfigWrite => Some(Mode::Ask),
            _ => None,
        }
    }

    /// Dangerous categories: closed when the observer is unavailable (fail-closed).
    #[must_use]
    pub fn is_dangerous(self) -> bool {
        matches!(
            self,
            Action::Delete | Action::Pay | Action::Shell | Action::Send | Action::ConfigWrite
        )
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Permission mode; strictness order `Allow < Ask < Deny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Allow,
    Ask,
    Deny,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Allow => "allow",
            Mode::Ask => "ask",
            Mode::Deny => "deny",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "allow" => Some(Mode::Allow),
            "ask" => Some(Mode::Ask),
            "deny" => Some(Mode::Deny),
            _ => None,
        }
    }
}

/// Rule level: from top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Global,
    Persona,
    Session,
}

impl Level {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Global => "global",
            Level::Persona => "persona",
            Level::Session => "session",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Level> {
        match s {
            "global" => Some(Level::Global),
            "persona" => Some(Level::Persona),
            "session" => Some(Level::Session),
            _ => None,
        }
    }
}

/// A `permissions` table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub level: Level,
    pub scope_id: i64,
    pub action: Action,
    pub mode: Mode,
}

impl Rule {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"scope": self.level.as_str(), "scope_id": self.scope_id,
               "action": self.action.as_str(), "mode": self.mode.as_str()})
    }
}

/// Raw rules of three levels for one action (as stored in the table, unprocessed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub global: Option<Mode>,
    pub persona: Option<Mode>,
    pub session: Option<Mode>,
}

/// Effective mode: the strictest of the three levels; a missing level does not participate. No rules at all — `None`.
#[must_use]
pub fn effective_mode(
    global: Option<Mode>,
    persona: Option<Mode>,
    session: Option<Mode>,
) -> Option<Mode> {
    [global, persona, session].into_iter().flatten().max()
}

/// Whether an entry with `mode` at `level` loosens the rules of higher levels: a lower level can only tighten.
#[must_use]
pub fn would_loosen(level: Level, mode: Mode, set: &RuleSet) -> bool {
    let above = match level {
        Level::Global => None,
        Level::Persona => set.global,
        Level::Session => effective_mode(set.global, set.persona, None),
    };
    above.is_some_and(|a| mode < a)
}

/// Set rows that loosen higher-level ones: bypassing them does not change the `effective_mode` result (maximum), but the observer
/// must be told about them (`permission_loosen`).
#[must_use]
pub fn loosening_levels(set: &RuleSet) -> Vec<Level> {
    let mut out = Vec::new();
    if let Some(m) = set.persona
        && would_loosen(Level::Persona, m, set)
    {
        out.push(Level::Persona);
    }
    if let Some(m) = set.session
        && would_loosen(Level::Session, m, set)
    {
        out.push(Level::Session);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mode::{Allow, Ask, Deny};

    /// Permissions table.
    #[test]
    fn permissions_tightening_table() {
        type Row = (Option<Mode>, Option<Mode>, Option<Mode>, Option<Mode>);
        let rows: [Row; 6] = [
            (Some(Allow), Some(Ask), None, Some(Ask)),
            (Some(Allow), None, Some(Deny), Some(Deny)),
            (Some(Deny), Some(Allow), None, Some(Deny)),
            (Some(Deny), None, Some(Allow), Some(Deny)),
            (Some(Ask), Some(Allow), Some(Ask), Some(Ask)),
            (None, Some(Allow), Some(Deny), Some(Deny)),
        ];
        for (g, p, s, want) in rows {
            assert_eq!(effective_mode(g, p, s), want, "{g:?}/{p:?}/{s:?}");
        }
        assert_eq!(effective_mode(None, None, None), None);
    }

    #[test]
    fn loosening_is_detected_by_level() {
        let set = RuleSet {
            global: Some(Ask),
            persona: None,
            session: Some(Allow),
        };
        assert!(would_loosen(Level::Session, Allow, &set));
        assert!(!would_loosen(Level::Session, Ask, &set));
        assert!(!would_loosen(Level::Session, Deny, &set));
        assert!(!would_loosen(Level::Global, Allow, &set));
        assert_eq!(loosening_levels(&set), vec![Level::Session]);
        let set = RuleSet {
            global: Some(Deny),
            persona: Some(Ask),
            session: Some(Ask),
        };
        assert_eq!(loosening_levels(&set), vec![Level::Persona, Level::Session]);
    }

    #[test]
    fn names_roundtrip_and_reject_unknown() {
        for a in Action::ALL {
            assert_eq!(Action::parse(a.as_str()), Some(a));
        }
        for m in [Allow, Ask, Deny] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        for l in [Level::Global, Level::Persona, Level::Session] {
            assert_eq!(Level::parse(l.as_str()), Some(l));
        }
        assert_eq!(Action::parse("Shell"), None);
        assert_eq!(Mode::parse("ALLOW"), None);
        assert_eq!(Level::parse(""), None);
    }
}
