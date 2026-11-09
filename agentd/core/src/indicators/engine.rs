//! Engine: polls enabled sources and passes frames to the transition store.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::{
    IndicatorError, IndicatorSource, IndicatorState, IndicatorStore, Observed, Reading, Sources,
    valid_id,
};
use crate::{FieldError, SettingsStore};

#[derive(Clone)]
pub struct IndicatorEngine {
    store: Arc<dyn IndicatorStore>,
    settings: Arc<dyn SettingsStore>,
    sources: Arc<std::sync::Mutex<Sources>>,
}

impl IndicatorEngine {
    #[must_use]
    pub fn new(
        store: Arc<dyn IndicatorStore>,
        settings: Arc<dyn SettingsStore>,
    ) -> IndicatorEngine {
        IndicatorEngine {
            store,
            settings,
            sources: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Registers a source; system sources plug in here as well.
    pub fn register(&self, source: Arc<dyn IndicatorSource>) {
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(source);
    }

    #[must_use]
    pub fn store(&self) -> &Arc<dyn IndicatorStore> {
        &self.store
    }

    /// The only frame entry point: sources and the test build go through here.
    pub async fn push(&self, id: &str, reading: Reading) -> Result<Observed, IndicatorError> {
        if !valid_id(id) {
            return Err(IndicatorError::Invalid(vec![FieldError::new(
                "id",
                "a-z, 0-9, _ . - up to 64 characters",
            )]));
        }
        self.store.observe(id.to_string(), reading).await
    }

    /// Identifiers enabled on the `indicators` page.
    async fn enabled(&self) -> Vec<String> {
        let Ok(doc) = self.settings.get("indicators").await else {
            return Vec::new();
        };
        doc.doc["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["enabled"] == Value::Bool(true))
                    .filter_map(|i| i["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One poll: enabled sources are read, disabled ones are not touched; an unavailable read yields `nodata`.
    pub async fn tick(&self) {
        let enabled = self.enabled().await;
        let sources: Sources = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for s in sources
            .iter()
            .filter(|s| enabled.iter().any(|e| e == s.id()))
        {
            let reading = s.read().await;
            if self.push(s.id(), reading).await.is_err() {
                tracing::warn!(
                    scope = "indicators",
                    event = "push_failed",
                    id = s.id(),
                    "frame not written"
                );
            }
        }
    }

    /// Background polling with period `every`.
    pub fn spawn(&self, every: Duration) {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            loop {
                t.tick().await;
                engine.tick().await;
            }
        });
    }
}

/// "No data" frame for a source that could not read its metric.
#[must_use]
pub fn nodata(text: &str) -> Reading {
    Reading {
        state: IndicatorState::Nodata,
        value: None,
        text: Some(text.to_string()),
    }
}
