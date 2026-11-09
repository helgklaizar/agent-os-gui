//! Unified search (`docs/reference/api.md` 5.5): index store port and `GET /search` query validation.
//! The index is lexical (FTS5); the user's query syntax is not executed — words are searched as plain text.

use serde_json::{Value, json};

use crate::{FieldError, PortFuture};

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 50;
pub const MAX_QUERY_CHARS: usize = 200;

/// Result entry: `kind` — `session | message | file | memory`, `ref` — where to navigate (`session_id`, `message_id`, `path`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub kind: String,
    pub source: &'static str,
    pub title: String,
    pub snippet: String,
    pub reference: Value,
}

impl SearchHit {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"kind": self.kind, "source": self.source, "title": self.title,
               "snippet": self.snippet, "ref": self.reference})
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SearchError {
    Unavailable,
}

/// Index store: reconciliation with sources and the query (implemented by `agentd-db`).
pub trait SearchStore: Send + Sync + 'static {
    fn search(
        &self,
        query: String,
        limit: usize,
    ) -> PortFuture<'_, Result<Vec<SearchHit>, SearchError>>;
}

/// Parsing the query string: `q` is required (1…200 characters without outer spaces), `limit` 1…50.
pub fn parse_query(
    q: Option<&str>,
    limit: Option<&str>,
) -> Result<(String, usize), Vec<FieldError>> {
    let mut errs = Vec::new();
    let text = q.map(str::trim).unwrap_or_default();
    if text.is_empty() || text.chars().count() > MAX_QUERY_CHARS {
        errs.push(FieldError::new("q", "a string of 1 to 200 characters"));
    }
    let limit = match limit {
        None => DEFAULT_LIMIT,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => {
                errs.push(FieldError::new("limit", "an integer from 1 to 50"));
                DEFAULT_LIMIT
            }
        },
    };
    if errs.is_empty() {
        Ok((text.to_string(), limit))
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_and_bounds() {
        assert_eq!(
            parse_query(Some(" brød "), None).unwrap(),
            ("brød".to_string(), DEFAULT_LIMIT)
        );
        assert_eq!(parse_query(Some("x"), Some("50")).unwrap().1, 50);
        for bad in [None, Some(""), Some("   ")] {
            assert!(parse_query(bad, None).is_err());
        }
        for l in ["0", "51", "-1", "a"] {
            assert!(parse_query(Some("x"), Some(l)).is_err(), "{l}");
        }
        assert!(parse_query(Some(&"é".repeat(201)), None).is_err());
    }
}
