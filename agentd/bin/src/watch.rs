//! Wiring of the model-based observer (A7-03). Off by default: enabled with `AGENTD_WATCH=1` (core unit or owner).
//! The model is `AGENTD_WATCH_MODEL` (default `claude-haiku-5-5`), the pulse period is `AGENTD_WATCH_HEARTBEAT_SECS` (60).

use std::sync::Arc;
use std::time::Duration;

use agentd_core::{
    ChatStore, ModelClassifier, ModelObserver, Observer, ProviderRegistry, SettingsStore,
    StoredAccounts, TreeStore, WATCH_DEFAULT_MODEL as DEFAULT_MODEL,
    WATCH_DEFAULT_TIMEOUT as DEFAULT_TIMEOUT,
};

pub(crate) struct Deps {
    pub providers: ProviderRegistry,
    pub tree: Arc<dyn TreeStore>,
    pub chat: Arc<dyn ChatStore>,
    pub settings: Arc<dyn SettingsStore>,
}

#[must_use]
pub(crate) fn enabled() -> bool {
    matches!(
        std::env::var("AGENTD_WATCH").as_deref(),
        Ok("1" | "on" | "true")
    )
}

/// Socket of the `agent-watch` process (`AGENTD_WATCH_SOCKET`); empty or unset means the core executes the rules itself.
#[must_use]
pub(crate) fn socket() -> Option<std::path::PathBuf> {
    std::env::var_os("AGENTD_WATCH_SOCKET")
        .filter(|s| !s.is_empty())
        .map(Into::into)
}

/// Wraps the rule-based observer with the model and starts the pulse; call inside the tokio runtime.
pub(crate) fn wrap(rules: Arc<dyn Observer>, d: Deps) -> Arc<dyn Observer> {
    let model = std::env::var("AGENTD_WATCH_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let every = std::env::var("AGENTD_WATCH_HEARTBEAT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(60);
    let classifier = ModelClassifier::new(
        d.providers,
        Arc::new(StoredAccounts {
            tree: d.tree,
            chat: d.chat,
            settings: d.settings,
        }),
        model,
        DEFAULT_TIMEOUT,
    );
    let obs = Arc::new(ModelObserver::new(rules, Arc::new(classifier)));
    obs.spawn_heartbeat(Duration::from_secs(every));
    tracing::info!(
        scope = "observer",
        event = "watch_enabled",
        "model-based observer enabled"
    );
    obs
}
