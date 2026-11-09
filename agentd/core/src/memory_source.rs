//! `memory_mcp` memory source: a `search` request through the core MCP client. Only the query text and the limit go to `memory_mcp`;
//! the answer is not copied to the DB; unavailability and slow answers degrade to local search.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Mcp, SearchHit};

/// `memory_mcp` request timeout, consecutive failures before a pause and pause length (assumptions, to be replaced by measurement).
pub const MEMORY_TIMEOUT: Duration = Duration::from_secs(2);
pub const MEMORY_MAX_FAILS: u32 = 3;
pub const MEMORY_PAUSE: Duration = Duration::from_secs(60);
/// MCP server name `memory_mcp` on the `connections` page.
const SERVER: &str = "memory_mcp";

#[derive(Default)]
struct State {
    fails: u32,
    skip_until: Option<Instant>,
}

#[derive(Clone)]
pub struct McpMemorySource {
    mcp: Mcp,
    state: Arc<Mutex<State>>,
}

impl McpMemorySource {
    #[must_use]
    pub fn new(mcp: Mcp) -> McpMemorySource {
        McpMemorySource {
            mcp,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn note(&self, ok: bool) {
        let mut s = self.state();
        if ok {
            *s = State::default();
        } else {
            s.fails += 1;
            if s.fails >= MEMORY_MAX_FAILS {
                s.skip_until = Some(Instant::now() + MEMORY_PAUSE);
                s.fails = 0;
            }
        }
    }

    /// Memory entries from `memory_mcp`; empty when unavailable. No extra fields in the request.
    pub async fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        if self.state().skip_until.is_some_and(|t| t > Instant::now()) {
            return Vec::new();
        }
        let call = async {
            let mut tool = self
                .mcp
                .resolve(&crate::mcp_tool_name(SERVER, "search"))
                .await?;
            tool.timeout = MEMORY_TIMEOUT;
            self.mcp
                .call(&tool, &json!({"query": query, "limit": limit}))
                .await
                .ok()
        };
        let res = tokio::time::timeout(MEMORY_TIMEOUT * 2, call).await;
        let hits = res
            .ok()
            .flatten()
            .filter(|r| !r.is_error)
            .map(|r| parse(&r.text));
        self.note(hits.is_some());
        hits.unwrap_or_default()
    }
}

/// `memory_mcp` answer: an array of `{title, snippet}`; foreign fields are dropped, string length is limited.
fn parse(text: &str) -> Vec<SearchHit> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .take(50)
        .filter_map(|i| {
            let cut =
                |k: &str, n: usize| i[k].as_str().map(|s| s.chars().take(n).collect::<String>());
            Some(SearchHit {
                kind: "memory".to_string(),
                source: "mcp_memory",
                title: cut("title", 200)?,
                snippet: cut("snippet", 400).unwrap_or_default(),
                reference: json!({"source": "mcp_memory"}),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_only_title_and_snippet() {
        let h = parse(r#"[{"title":"A","snippet":"B","secret":"x"},{"snippet":"no title"},5]"#);
        assert_eq!(h.len(), 1);
        assert_eq!(
            (h[0].title.as_str(), h[0].snippet.as_str(), h[0].source),
            ("A", "B", "mcp_memory")
        );
        assert!(parse("not json").is_empty());
    }
}
