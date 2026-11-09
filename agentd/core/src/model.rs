//! Models: provider trait, normalized events, account selection. No I/O here:
//! HTTP clients and stream parsing are in the `agentd-providers` crate.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::PortFuture;

/// Provider kind by account type (`accounts.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    OpenAiCompat,
    /// Local OpenAI-compatible server (Ollama, llama.cpp): the key is optional, no limits.
    Local,
    /// Test provider (`test-support` build only).
    Scripted,
}

impl ProviderKind {
    /// Account type → provider kind; mail and messengers do not provide models.
    #[must_use]
    pub fn from_account_type(t: &str) -> Option<ProviderKind> {
        match t {
            "anthropic" => Some(ProviderKind::Anthropic),
            "openai_compat" | "openai" => Some(ProviderKind::OpenAiCompat),
            "local" | "ollama" => Some(ProviderKind::Local),
            "scripted" => Some(ProviderKind::Scripted),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_local(self) -> bool {
        self == ProviderKind::Local
    }
}

/// Model account: what the provider needs. The secret value is not here — only a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAccount {
    pub id: i64,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: Option<String>,
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
}

/// History turn for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: TurnRole,
    pub text: String,
}

/// A tool the model sees: name, description, argument schema (JSON Schema).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub account: ModelAccount,
    pub model: String,
    pub system: Option<String>,
    pub history: Vec<Turn>,
    pub max_tokens: u32,
    /// Tool catalog: built-in actions and MCP tools; empty — a model without tools.
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// Limit window from response headers: how much is allowed, how much remains, when it resets (unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitWindow {
    pub limit: u64,
    pub remaining: u64,
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RateLimit {
    pub requests: Option<LimitWindow>,
    pub tokens: Option<LimitWindow>,
}

impl RateLimit {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_none() && self.tokens.is_none()
    }
}

/// A tool call the model asked for (`tool_use` for Anthropic, `tool_calls` for OpenAI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// Call identifier from the provider.
    pub id: String,
    /// Tool name; mapped to an action by `action_for_tool`.
    pub name: String,
    /// Call arguments (JSON object).
    pub args: serde_json::Value,
}

/// Normalized provider error. `code`: `unauthorized`, `rate_limited`, `overloaded`, `upstream`, `timeout`,
/// `network`, `bad_response`, `no_key`, `no_model`, `unsupported`. The provider's message text is not included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub code: &'static str,
    pub retryable: bool,
    pub status: Option<u16>,
}

impl ProviderError {
    #[must_use]
    pub fn new(code: &'static str, retryable: bool) -> ProviderError {
        ProviderError {
            code,
            retryable,
            status: None,
        }
    }

    /// Provider HTTP response code → normalized error.
    #[must_use]
    pub fn from_status(status: u16) -> ProviderError {
        let (code, retryable) = match status {
            401 | 403 => ("unauthorized", false),
            429 => ("rate_limited", true),
            529 => ("overloaded", true),
            500..=599 => ("upstream", true),
            _ => ("bad_response", false),
        };
        ProviderError {
            code,
            retryable,
            status: Some(status),
        }
    }
}

/// Response stream event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    Delta(String),
    Usage(Usage),
    RateLimit(RateLimit),
    Error(ProviderError),
    /// The model asks to perform an action: the core passes it through permissions, the observer and the executor.
    ToolCall(ToolCall),
}

/// Model provider. Cancellation is dropping the future `stream`: the connection is closed.
pub trait ModelProvider: Send + Sync + 'static {
    /// Sends events to `tx` until the end of the reply; an error is an `Error` event and return. The key is taken from the secret store
    /// at request time.
    fn stream<'a>(
        &'a self,
        req: ModelRequest,
        tx: mpsc::Sender<ProviderEvent>,
    ) -> PortFuture<'a, ()>;

    /// Light check of key and address (`connected` or `error` in the account status).
    fn check<'a>(&'a self, account: &'a ModelAccount) -> PortFuture<'a, Result<(), ProviderError>>;
}

/// Provider registry by kind.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    map: HashMap<ProviderKind, Arc<dyn ModelProvider>>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn new() -> ProviderRegistry {
        ProviderRegistry::default()
    }

    #[must_use]
    pub fn with(mut self, kind: ProviderKind, p: Arc<dyn ModelProvider>) -> ProviderRegistry {
        self.map.insert(kind, p);
        self
    }

    #[must_use]
    pub fn get(&self, kind: ProviderKind) -> Option<Arc<dyn ModelProvider>> {
        self.map.get(&kind).cloned()
    }

    #[must_use]
    pub fn kinds(&self) -> Vec<ProviderKind> {
        self.map.keys().copied().collect()
    }
}

/// Session model account: the session's `cfg.model`, otherwise the effective persona's `personas.model_acc`; neither —
/// `None` (a reply `error` with code `no_model`).
#[must_use]
pub fn pick_account(cfg_model: Option<i64>, persona_model_acc: Option<i64>) -> Option<i64> {
    cfg_model.or(persona_model_acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_session_cfg() {
        assert_eq!(pick_account(Some(2), Some(3)), Some(2));
        assert_eq!(pick_account(None, Some(3)), Some(3));
        assert_eq!(pick_account(None, None), None);
    }

    #[test]
    fn kinds_by_account_type() {
        for (t, k) in [
            ("anthropic", ProviderKind::Anthropic),
            ("openai_compat", ProviderKind::OpenAiCompat),
            ("ollama", ProviderKind::Local),
            ("local", ProviderKind::Local),
        ] {
            assert_eq!(ProviderKind::from_account_type(t), Some(k), "{t}");
        }
        for t in ["wa", "tg", "gmail", "", "Anthropic"] {
            assert_eq!(ProviderKind::from_account_type(t), None, "{t}");
        }
    }

    #[test]
    fn status_map_to_normalized_codes() {
        for (s, code, retry) in [
            (401, "unauthorized", false),
            (403, "unauthorized", false),
            (429, "rate_limited", true),
            (529, "overloaded", true),
            (500, "upstream", true),
            (503, "upstream", true),
            (400, "bad_response", false),
            (404, "bad_response", false),
        ] {
            let e = ProviderError::from_status(s);
            assert_eq!(
                (e.code, e.retryable, e.status),
                (code, retry, Some(s)),
                "{s}"
            );
        }
    }
}
