//! Model account check with a light provider request: `connected` or `error` in the account status.

use std::sync::Arc;

use serde_json::Value;

use super::prompt::account_settings;
use super::{ChatService, Inner};
use crate::{ModelAccount, ProviderKind};

async fn check(inner: &Inner, id: i64) {
    let Ok(Some(row)) = inner.store.account(id).await else {
        return;
    };
    let Some(kind) = ProviderKind::from_account_type(&row.kind) else {
        return; // mail and messengers: their connection maintains the status
    };
    let Some(provider) = inner.providers.get(kind) else {
        return;
    };
    let doc = inner
        .settings
        .get("models")
        .await
        .map(|d| d.doc)
        .unwrap_or_default();
    let s = account_settings(&doc, id);
    let account = ModelAccount {
        id,
        kind,
        label: row.label,
        base_url: s.base_url,
        secret_ref: row
            .secret_ref
            .or_else(|| Some(format!("provider.{}", row.kind))),
    };
    let status = match provider.check(&account).await {
        Ok(()) => "connected",
        // No key yet: nothing to check with, the status stays as it was.
        Err(e) if e.code == "no_key" => return,
        Err(e) => {
            tracing::info!(
                scope = "chat",
                event = "account_check_failed",
                account_id = id,
                code = e.code,
                "account check failed"
            );
            "error"
        }
    };
    if inner.tree.set_account_status(id, status).await.is_err() {
        tracing::debug!(
            scope = "chat",
            event = "account_gone",
            "account vanished before the status was written"
        );
    }
}

impl ChatService {
    /// Checks the account in the background (after its secret is written).
    pub fn spawn_check(&self, account_id: i64) {
        let inner: Arc<Inner> = self.inner.clone();
        tokio::spawn(async move { check(&inner, account_id).await });
    }

    /// Checks the accounts listed on the `models` page (after it changes).
    pub fn spawn_check_models(&self, doc: &Value) {
        let ids = doc["accounts"]
            .as_object()
            .map(|m| {
                m.keys()
                    .filter_map(|k| k.parse::<i64>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            self.spawn_check(id);
        }
    }
}
