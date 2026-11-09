//! `GET /search?q=&limit=20` (docs/reference/api.md 5.5): unified Ctrl+K search over sessions, messages and memory files.

use std::collections::HashMap;
use std::sync::Arc;

use agentd_core::{SearchError, parse_search_query};
use axum::extract::{RawQuery, State};
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::ApiState;
use crate::tree::{invalid, ok};

/// Query string with percent-encoding (the query text is Cyrillic and punctuation).
fn pairs(raw: Option<String>) -> HashMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn handler(
    State(state): State<Arc<ApiState>>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let store = state
        .services()
        .and_then(|s| s.search.clone())
        .ok_or_else(|| ApiError::core_down("Search unavailable"))?;
    let q = pairs(raw);
    let (text, limit) = parse_search_query(
        q.get("q").map(String::as_str),
        q.get("limit").map(String::as_str),
    )
    .map_err(invalid)?;
    let mut hits = store
        .search(text.clone(), limit)
        .await
        .map_err(|SearchError::Unavailable| ApiError::core_down("Search unavailable"))?;
    // Memory source: `local` (default) does not query memory_mcp; `mcp_memory` and `both` add its memory records.
    let source = match state.services().and_then(|s| s.settings.clone()) {
        Some(s) => s
            .get("memory")
            .await
            .ok()
            .and_then(|d| d.doc["source"].as_str().map(str::to_string)),
        None => None,
    };
    if matches!(source.as_deref(), Some("mcp_memory" | "both"))
        && let Some(memory_mcp) = state.services().and_then(|s| s.memory_mcp.clone())
    {
        hits.extend(memory_mcp.search(&text, limit).await);
        hits.truncate(limit);
    }
    Ok(ok(
        json!({"items": hits.iter().map(agentd_core::SearchHit::to_json).collect::<Vec<Value>>()}),
    ))
}

pub(crate) fn route() -> MethodRouter<Arc<ApiState>> {
    get(handler)
}
