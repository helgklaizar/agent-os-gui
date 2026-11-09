//! Agent action catalog: mapping tool calls to actions, the call target and executors.
//! Executors without capabilities answer with an honest error (`executor_unavailable`, `no_connector`), not success.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::{Action, Notifier, PortFuture, SettingsError, SettingsStore};

/// Provider tool name → action. Canonical names match action names; aliases are common
/// names from model tool catalogs. An unknown name — `None` (the call is rejected, permissions are not bypassed).
#[must_use]
pub fn action_for_tool(name: &str) -> Option<Action> {
    if let Some(a) = Action::parse(name) {
        return Some(a);
    }
    match name {
        "bash" | "run_shell" | "run_command" | "execute_command" | "terminal" => {
            Some(Action::Shell)
        }
        "send_message" | "send_email" | "send_mail" => Some(Action::Send),
        "delete_file" | "remove_file" | "delete_message" => Some(Action::Delete),
        "make_payment" | "pay_invoice" => Some(Action::Pay),
        "browser" | "browse_url" | "open_url" | "navigate" => Some(Action::Browse),
        "read_file" | "list_files" | "search" => Some(Action::Read),
        "write_memory" | "remember" => Some(Action::MemoryWrite),
        "write_config" | "update_settings" => Some(Action::ConfigWrite),
        _ => None,
    }
}

/// Call target for the log and the card: the first of the fields `target`, `command`, `url`, `path`, `to`, `page`.
#[must_use]
pub fn target_of(args: &Value) -> String {
    ["target", "command", "url", "path", "to", "page"]
        .iter()
        .find_map(|k| args.get(k).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// What the executor receives as input.
#[derive(Debug, Clone)]
pub struct ActionCall {
    pub action: Action,
    pub target: String,
    pub args: Value,
    pub session_id: i64,
}

/// Executor refusal; `code` goes to the model as a tool error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecError {
    pub code: &'static str,
}

pub trait ActionExecutor: Send + Sync + 'static {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>>;
}

/// No executor: the action is not performed, the model gets `code`.
pub struct Unavailable(pub &'static str);

impl ActionExecutor for Unavailable {
    fn execute<'a>(&'a self, _call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        let code = self.0;
        Box::pin(async move { Err(ExecError { code }) })
    }
}

/// `config_write`: edits a settings page by the same path as the API (`args = {page, patch}`).
pub struct ConfigExecutor {
    settings: Arc<dyn SettingsStore>,
    notifier: Arc<dyn Notifier>,
}

impl ConfigExecutor {
    #[must_use]
    pub fn new(settings: Arc<dyn SettingsStore>, notifier: Arc<dyn Notifier>) -> ConfigExecutor {
        ConfigExecutor { settings, notifier }
    }
}

impl ActionExecutor for ConfigExecutor {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        Box::pin(async move {
            let invalid = ExecError {
                code: "invalid_args",
            };
            let page = call
                .args
                .get("page")
                .and_then(Value::as_str)
                .ok_or(invalid.clone())?;
            let patch = call
                .args
                .get("patch")
                .filter(|p| p.is_object())
                .ok_or(invalid.clone())?;
            let cur = self.settings.get(page).await.map_err(|_| invalid.clone())?;
            match self.settings.patch(page, patch.clone(), cur.rev).await {
                Ok(saved) => {
                    self.notifier
                        .publish("settings", json!({"page": page, "rev": saved.rev}));
                    Ok(json!({"page": page, "rev": saved.rev}))
                }
                Err(SettingsError::Invalid(_) | SettingsError::UnknownPage) => Err(invalid),
                Err(SettingsError::ReadOnly) => Err(ExecError { code: "read_only" }),
                Err(SettingsError::Stale { .. }) => Err(ExecError { code: "conflict" }),
                Err(SettingsError::Unavailable) => Err(ExecError {
                    code: "unavailable",
                }),
            }
        })
    }
}

/// Executors by action; no executor — `executor_unavailable`.
#[derive(Clone, Default)]
pub struct Executors {
    map: HashMap<Action, Arc<dyn ActionExecutor>>,
}

impl Executors {
    /// Set 0.1: settings are executed, external connectors (`send`, `pay`, `delete`) are not connected yet
    /// (`no_connector`), the agent terminal is `executor_unavailable`.
    #[must_use]
    pub fn standard(settings: Arc<dyn SettingsStore>, notifier: Arc<dyn Notifier>) -> Executors {
        let mut e = Executors::default();
        for a in [Action::Send, Action::Pay, Action::Delete] {
            e = e.with(a, Arc::new(Unavailable("no_connector")));
        }
        e.with(
            Action::ConfigWrite,
            Arc::new(ConfigExecutor::new(settings, notifier)),
        )
    }

    #[must_use]
    pub fn with(mut self, action: Action, exec: Arc<dyn ActionExecutor>) -> Executors {
        self.map.insert(action, exec);
        self
    }

    #[must_use]
    pub fn get(&self, action: Action) -> Arc<dyn ActionExecutor> {
        self.map
            .get(&action)
            .cloned()
            .unwrap_or_else(|| Arc::new(Unavailable("executor_unavailable")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_map_to_actions() {
        for a in Action::ALL {
            assert_eq!(action_for_tool(a.as_str()), Some(a));
        }
        for (n, a) in [
            ("bash", Action::Shell),
            ("send_email", Action::Send),
            ("delete_file", Action::Delete),
            ("make_payment", Action::Pay),
            ("open_url", Action::Browse),
            ("read_file", Action::Read),
            ("remember", Action::MemoryWrite),
            ("update_settings", Action::ConfigWrite),
        ] {
            assert_eq!(action_for_tool(n), Some(a), "{n}");
        }
        for n in [
            "",
            "Shell",
            "rm",
            "grant_permission",
            "set_permission",
            "issue_token",
        ] {
            assert_eq!(action_for_tool(n), None, "{n}");
        }
    }

    #[test]
    fn target_prefers_named_fields() {
        assert_eq!(target_of(&json!({"command": "ls", "url": "u"})), "ls");
        assert_eq!(target_of(&json!({"target": "t", "command": "ls"})), "t");
        assert_eq!(target_of(&json!({"x": 1})), "");
        assert_eq!(target_of(&json!({"target": 5})), "");
    }
}
