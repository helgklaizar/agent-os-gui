//! Observer (`docs/reference/spec.md` 3.6): interface and a rule-based implementation inside the core. A separate daemon
//! `agent-watch` plugs in through the same interface (crate `agentd-watch`).

mod rules;
mod shell;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Action, PortFuture};

pub use rules::{RuleError, RuleSetFile};

const DANGEROUS_DEFAULT: &str = include_str!("../../rules/dangerous_cmd.toml");
const INJECTION_DEFAULT: &str = include_str!("../../rules/injection.toml");

/// Observer availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverStatus {
    Available,
    Unavailable,
}

/// An observer finding before it is written to `observer_findings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// `dangerous_cmd` or `injection`.
    pub kind: &'static str,
    /// `low|med|high`.
    pub severity: &'static str,
    /// `blocked` (without confirmation) or `asked` (a card).
    pub verdict: &'static str,
    /// Name of the rule that fired (not the text under review).
    pub rule: String,
}

pub trait Observer: Send + Sync + 'static {
    fn status(&self) -> ObserverStatus;
    /// Action check: for `shell` the command is in `target`; other actions are not covered by the command rules.
    fn check_action(&self, action: Action, target: &str) -> Option<Hit>;
    /// Check of incoming untrusted text for injection markers.
    fn check_text(&self, text: &str) -> Option<Hit>;
    /// Second line: model classification of the action. Called only after the rules found nothing;
    /// a model error here marks the observer `Unavailable` and returns a request finding (fail-closed). The rule-based
    /// observer has no model.
    fn classify<'a>(&'a self, _action: Action, _target: &'a str) -> PortFuture<'a, Option<Hit>> {
        Box::pin(async { None })
    }
    /// Test build only: simulated unavailability. `false` — the implementation cannot do it.
    #[cfg(feature = "test-support")]
    fn force_available(&self, _available: bool) -> bool {
        false
    }
}

/// Rule-based observer.
pub struct RuleObserver {
    commands: RuleSetFile,
    injection: rules::Markers,
    available: AtomicBool,
}

impl RuleObserver {
    /// Rules from TOML texts.
    pub fn from_toml(commands: &str, injection: &str) -> Result<RuleObserver, RuleError> {
        Ok(RuleObserver {
            commands: RuleSetFile::parse(commands)?,
            injection: rules::Markers::parse(injection)?,
            available: AtomicBool::new(true),
        })
    }

    /// Rules from `dir/dangerous_cmd.toml` and `dir/injection.toml`; a missing file is replaced by the built-in copy,
    /// a broken file is a startup error (no silent fallback to defaults).
    pub fn load(dir: Option<&Path>) -> Result<RuleObserver, RuleError> {
        let read = |name: &str, default: &str| -> Result<String, RuleError> {
            let Some(dir) = dir else {
                return Ok(default.to_string());
            };
            match std::fs::read_to_string(dir.join(name)) {
                Ok(s) => Ok(s),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default.to_string()),
                Err(_) => Err(RuleError::Unreadable(name.to_string())),
            }
        };
        RuleObserver::from_toml(
            &read("dangerous_cmd.toml", DANGEROUS_DEFAULT)?,
            &read("injection.toml", INJECTION_DEFAULT)?,
        )
    }
}

impl Observer for RuleObserver {
    fn status(&self) -> ObserverStatus {
        if self.available.load(Ordering::SeqCst) {
            ObserverStatus::Available
        } else {
            ObserverStatus::Unavailable
        }
    }

    fn check_action(&self, action: Action, target: &str) -> Option<Hit> {
        if action != Action::Shell {
            return None;
        }
        self.commands.evaluate(target).map(|r| Hit {
            kind: "dangerous_cmd",
            severity: r.severity,
            verdict: r.verdict,
            rule: r.name.clone(),
        })
    }

    fn check_text(&self, text: &str) -> Option<Hit> {
        self.injection.find(text).map(|m| Hit {
            kind: "injection",
            severity: self.injection.severity,
            verdict: "blocked",
            rule: m,
        })
    }

    #[cfg(feature = "test-support")]
    fn force_available(&self, available: bool) -> bool {
        self.available.store(available, Ordering::SeqCst);
        true
    }
}
