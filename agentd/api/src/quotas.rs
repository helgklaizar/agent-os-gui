//! `GET /quotas` and the `bar.quota` topic (docs/reference/api.md 4.4, 5.6): accounting by actual usage.

use std::sync::Arc;

use agentd_core::QuotaStore;
use axum::extract::{RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::Value;

use crate::chat;
use crate::error::ApiError;
use crate::hub::TopicSource;
use crate::state::ApiState;
use crate::tree::{ok, parse_id};

fn store(state: &ApiState) -> Result<Arc<dyn QuotaStore>, ApiError> {
    state
        .services()
        .and_then(|s| s.quotas.clone())
        .ok_or_else(|| ApiError::core_down("Usage accounting unavailable"))
}

async fn list(
    State(state): State<Arc<ApiState>>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let session = raw
        .as_deref()
        .unwrap_or_default()
        .split('&')
        .find_map(|p| p.strip_prefix("session_id="))
        .map(parse_id)
        .transpose()?;
    if let Some(id) = session {
        chat::store(&state)?
            .context(id)
            .await
            .map_err(chat::map_err)?;
    }
    let items = store
        .quotas(session)
        .await
        .ok_or_else(|| ApiError::core_down("Usage accounting unavailable"))?;
    Ok(ok(items))
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    get(list)
}

/// `snapshot` of the `bar.quota` topic: `{items: [Quota]}`.
pub(crate) struct QuotaTopic(pub Arc<dyn QuotaStore>);

impl TopicSource for QuotaTopic {
    fn snapshot<'a>(&'a self, _topic: &'a str) -> agentd_core::PortFuture<'a, Option<Value>> {
        Box::pin(async move { self.0.quotas(None).await })
    }
}
