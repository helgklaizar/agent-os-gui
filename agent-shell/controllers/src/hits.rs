//! Unified search results: parsing `SearchHit` from `GET /search` (docs/reference/api.md) and the jump target.
//!
//! Core text is shown only through `*_markup` (Pango escaping, `text::escape_markup`).

use serde_json::Value;

use crate::text::escape_markup;

/// Result kind; the variant order is the group order in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HitKind {
    Session,
    Message,
    Mail,
    File,
    Memory,
}

impl HitKind {
    pub fn parse(s: &str) -> Option<HitKind> {
        match s {
            "session" => Some(HitKind::Session),
            "message" => Some(HitKind::Message),
            "mail" => Some(HitKind::Mail),
            "file" => Some(HitKind::File),
            "memory" => Some(HitKind::Memory),
            _ => None,
        }
    }
}

/// Where the entry comes from: the local index or the MCP `memory_mcp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HitSource {
    Local,
    McpMemory,
}

/// Where to go on Enter; target owners handle [`crate::BusEvent::OpenHit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    Session {
        session_id: i64,
        message_id: Option<i64>,
    },
    Mail {
        id: i64,
    },
    File {
        path: String,
    },
    Memory {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: HitKind,
    pub source: HitSource,
    pub title: String,
    pub snippet: String,
    /// `None` means the core sent no link; the result is visible but cannot be opened.
    pub target: Option<HitTarget>,
}

impl Hit {
    pub fn title_markup(&self) -> String {
        escape_markup(&self.title)
    }

    pub fn snippet_markup(&self) -> String {
        escape_markup(&self.snippet)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HitError {
    #[error("search response not parsed: {0}")]
    Invalid(String),
}

fn target(kind: HitKind, r: Option<&Value>) -> Option<HitTarget> {
    let r = r?;
    let int = |k: &str| r.get(k).and_then(Value::as_i64);
    match kind {
        HitKind::Session => Some(HitTarget::Session {
            session_id: int("session_id")?,
            message_id: None,
        }),
        HitKind::Message => Some(HitTarget::Session {
            session_id: int("session_id")?,
            message_id: int("message_id"),
        }),
        HitKind::Mail => Some(HitTarget::Mail { id: int("id")? }),
        HitKind::File => Some(HitTarget::File {
            path: r.get("path")?.as_str()?.to_string(),
        }),
        HitKind::Memory => {
            let id = r.get("id")?;
            let id = id
                .as_str()
                .map(str::to_string)
                .or_else(|| id.as_i64().map(|n| n.to_string()))?;
            Some(HitTarget::Memory { id })
        }
    }
}

/// Parses `{ "items": [SearchHit] }`; any unparseable item fails the whole reply instead of being silently skipped.
pub fn parse_hits(body: &Value) -> Result<Vec<Hit>, HitError> {
    let bad = |m: String| HitError::Invalid(m);
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("items array missing".into()))?;
    items
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let kind = h
                .get("kind")
                .and_then(Value::as_str)
                .and_then(HitKind::parse)
                .ok_or_else(|| bad(format!("items[{i}].kind")))?;
            // docs/reference/api.md: `source` is required; a missing one is not replaced with `local`.
            let source = match h.get("source").and_then(Value::as_str) {
                Some("local") => HitSource::Local,
                Some("mcp_memory") => HitSource::McpMemory,
                _ => return Err(bad(format!("items[{i}].source"))),
            };
            let title = h
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| bad(format!("items[{i}].title")))?;
            let snippet = h.get("snippet").and_then(Value::as_str).unwrap_or_default();
            Ok(Hit {
                kind,
                source,
                title: title.to_string(),
                snippet: snippet.to_string(),
                target: target(kind, h.get("ref")),
            })
        })
        .collect()
}
