//! Browser command result (`ToolResult`, `docs/reference/api.md` 6): body parsing, output truncation.

use serde_json::{Value, json};

use crate::FieldError;

/// `output` limit, bytes (`docs/reference/api.md` 5.2.1); the rest is truncated with `truncated = true`.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Command result (`ToolResult` schema).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub request_id: String,
    /// `browser` or `user`.
    pub kind: String,
    /// `ok|error|timeout|cancelled|denied`.
    pub status: String,
    pub output: Option<String>,
    pub truncated: bool,
    /// Output size before truncation, bytes.
    pub output_bytes: Option<usize>,
    pub content_type: Option<String>,
    pub url: Option<String>,
    pub http_status: Option<i64>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

impl ToolResult {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"request_id": self.request_id, "kind": self.kind, "status": self.status,
               "output": self.output, "truncated": self.truncated, "output_bytes": self.output_bytes,
               "content_type": self.content_type, "url": self.url, "http_status": self.http_status,
               "error": self.error, "duration_ms": self.duration_ms})
    }
}

fn cut_utf8(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Parsing the `POST /sessions/{id}/tool-results` body: unknown fields and wrong types are field errors; output longer than
/// 256 KiB is truncated (`truncated`, `output_bytes` is the original size).
pub fn parse_tool_result(body: &Value) -> Result<ToolResult, Vec<FieldError>> {
    let Some(o) = body.as_object() else {
        return Err(vec![FieldError::new("", "an object is expected")]);
    };
    const KNOWN: [&str; 11] = [
        "request_id",
        "kind",
        "status",
        "output",
        "truncated",
        "output_bytes",
        "content_type",
        "url",
        "http_status",
        "error",
        "duration_ms",
    ];
    let mut errs: Vec<FieldError> = o
        .keys()
        .filter(|k| !KNOWN.contains(&k.as_str()))
        .map(|k| FieldError::new(k, "unknown field"))
        .collect();
    let text = |k: &str, errs: &mut Vec<FieldError>| match o.get(k) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            errs.push(FieldError::new(k, "a string is required"));
            None
        }
    };
    let int = |k: &str, errs: &mut Vec<FieldError>| match o.get(k) {
        None | Some(Value::Null) => None,
        Some(v) => v.as_i64().filter(|n| *n >= 0).or_else(|| {
            errs.push(FieldError::new(k, "a non-negative integer is required"));
            None
        }),
    };
    let request_id = text("request_id", &mut errs).filter(|s| !s.is_empty() && s.len() <= 200);
    if request_id.is_none() && !errs.iter().any(|e| e.field == "request_id") {
        errs.push(FieldError::new(
            "request_id",
            "a string up to 200 characters is required",
        ));
    }
    let kind = text("kind", &mut errs).filter(|k| matches!(k.as_str(), "browser" | "user"));
    if kind.is_none() {
        errs.push(FieldError::new("kind", "allowed: browser or user"));
    }
    let status = text("status", &mut errs).filter(|s| {
        matches!(
            s.as_str(),
            "ok" | "error" | "timeout" | "cancelled" | "denied"
        )
    });
    if status.is_none() {
        errs.push(FieldError::new(
            "status",
            "allowed: ok, error, timeout, cancelled, denied",
        ));
    }
    let mut output = text("output", &mut errs);
    let mut truncated = match o.get("truncated") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            errs.push(FieldError::new("truncated", "true or false is required"));
            false
        }
    };
    let mut output_bytes = int("output_bytes", &mut errs).and_then(|n| usize::try_from(n).ok());
    if let Some(out) = &output
        && out.len() > MAX_OUTPUT_BYTES
    {
        output_bytes = Some(out.len());
        truncated = true;
        output = Some(cut_utf8(out, MAX_OUTPUT_BYTES).to_string());
    }
    let res = ToolResult {
        request_id: request_id.unwrap_or_default(),
        kind: kind.unwrap_or_default(),
        status: status.unwrap_or_default(),
        output,
        truncated,
        output_bytes,
        content_type: text("content_type", &mut errs),
        url: text("url", &mut errs),
        http_status: int("http_status", &mut errs),
        error: text("error", &mut errs),
        duration_ms: int("duration_ms", &mut errs),
    };
    if errs.is_empty() { Ok(res) } else { Err(errs) }
}
