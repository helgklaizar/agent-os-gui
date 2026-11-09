//! Tool types as the controllers see them (S6, api.md sections 4.4, 5.2.1, 5.8): links, browser command, tool
//! result, agent terminal frames, tasks and runs. Unknown fields are ignored; wrong shape - `None`.

use serde_json::{Value, json};

use crate::browser_addr::is_loadable;

/// Bookmark or quick link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub title: String,
    pub url: String,
}

impl Link {
    pub fn json(&self) -> Value {
        json!({ "title": self.title, "url": self.url })
    }
}

/// Link list from a core reply; links whose address is not `http` or `https` are dropped.
pub fn parse_links(v: &Value) -> Vec<Link> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let title = l.get("title")?.as_str()?.to_string();
                    let url = l.get("url")?.as_str()?.to_string();
                    (is_loadable(&url) && !title.trim().is_empty()).then_some(Link { title, url })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Agent command to the browser (`browser.command` frame of the `session.<id>` topic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCommand {
    pub request_id: String,
    pub command: String,
    pub url: String,
}

pub fn parse_browser_command(data: &Value) -> Option<BrowserCommand> {
    let text = |k: &str| data.get(k)?.as_str().map(str::to_string);
    Some(BrowserCommand {
        request_id: text("request_id").filter(|s| !s.is_empty() && s.len() <= 200)?,
        command: text("command")?,
        url: text("url").unwrap_or_default(),
    })
}

/// Browser command result for `POST /sessions/{id}/tool-results` (api.md section 5.2.1, `kind = browser`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub request_id: String,
    /// `ok`, `error`, `timeout`, `cancelled`, `denied`.
    pub status: String,
    pub output: Option<String>,
    pub truncated: bool,
    pub output_bytes: Option<u64>,
    pub url: Option<String>,
    pub http_status: Option<i64>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
}

/// Output longer than this is truncated (working value from api.md section 5.2.1).
pub const OUTPUT_LIMIT: usize = 256 * 1024;

impl ToolResult {
    pub fn failed(request_id: &str, status: &str, error: &str) -> ToolResult {
        ToolResult {
            request_id: request_id.to_string(),
            status: status.to_string(),
            output: None,
            truncated: false,
            output_bytes: None,
            url: None,
            http_status: None,
            error: Some(error.to_string()),
            duration_ms: None,
        }
    }

    /// Success with page text: output is truncated at a character boundary to [`OUTPUT_LIMIT`] bytes.
    pub fn page(request_id: &str, url: &str, http_status: i64, text: &str, ms: u64) -> ToolResult {
        let total = text.len();
        let mut end = total.min(OUTPUT_LIMIT);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        ToolResult {
            request_id: request_id.to_string(),
            status: "ok".to_string(),
            output: Some(text[..end].to_string()),
            truncated: end < total,
            output_bytes: Some(total as u64),
            url: Some(url.to_string()),
            http_status: Some(http_status),
            error: None,
            duration_ms: Some(ms),
        }
    }

    pub fn json(&self) -> Value {
        let mut v = json!({
            "request_id": self.request_id, "kind": "browser", "status": self.status, "truncated": self.truncated,
        });
        let opt = |v: &mut Value, k: &str, x: Option<Value>| {
            if let Some(x) = x {
                v[k] = x;
            }
        };
        opt(&mut v, "output", self.output.clone().map(Value::from));
        opt(&mut v, "output_bytes", self.output_bytes.map(Value::from));
        opt(&mut v, "url", self.url.clone().map(Value::from));
        opt(&mut v, "http_status", self.http_status.map(Value::from));
        opt(&mut v, "error", self.error.clone().map(Value::from));
        opt(&mut v, "duration_ms", self.duration_ms.map(Value::from));
        v
    }
}

/// Agent terminal frames (api.md section 4.4): the core executes, the GUI only shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermFrame {
    Command {
        request_id: String,
        command: String,
        cwd: String,
    },
    Output {
        request_id: String,
        seq: u64,
        stderr: bool,
        data: String,
    },
    Status {
        request_id: String,
        status: String,
        exit_code: Option<i64>,
        duration_ms: Option<u64>,
    },
}

/// `session.<id>` event frame with `data.kind = terminal.*`.
pub fn parse_term_frame(data: &Value) -> Option<TermFrame> {
    let kind = data.get("kind")?.as_str()?;
    let rid = || {
        data.get("request_id")?
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 200)
            .map(str::to_string)
    };
    match kind {
        "terminal.command" => Some(TermFrame::Command {
            request_id: rid()?,
            command: data.get("command")?.as_str()?.to_string(),
            cwd: data
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "terminal.output" => Some(TermFrame::Output {
            request_id: rid()?,
            seq: data.get("seq")?.as_u64()?,
            stderr: data.get("stream")?.as_str()? == "stderr",
            data: data.get("data")?.as_str()?.to_string(),
        }),
        "terminal.status" => Some(TermFrame::Status {
            request_id: rid()?,
            status: data.get("status")?.as_str()?.to_string(),
            exit_code: data.get("exit_code").and_then(Value::as_i64),
            duration_ms: data.get("duration_ms").and_then(Value::as_u64),
        }),
        _ => None,
    }
}

/// Session task (`Task`, api.md section 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub id: i64,
    /// `sched` | `deleg`.
    pub kind: String,
    pub title: String,
    pub cron: Option<String>,
    pub tz: String,
    /// `active` | `paused` | `done`.
    pub state: String,
}

pub fn parse_task(v: &Value) -> Option<TaskItem> {
    let kind = v
        .get("kind")?
        .as_str()
        .filter(|k| matches!(*k, "sched" | "deleg"))?;
    let state = v
        .get("state")?
        .as_str()
        .filter(|s| matches!(*s, "active" | "paused" | "done"))?;
    Some(TaskItem {
        id: v.get("id")?.as_i64()?,
        kind: kind.to_string(),
        title: v.get("title")?.as_str()?.to_string(),
        cron: v.get("cron").and_then(Value::as_str).map(str::to_string),
        tz: v
            .get("tz")
            .and_then(Value::as_str)
            .unwrap_or("Asia/Jerusalem")
            .to_string(),
        state: state.to_string(),
    })
}

pub fn parse_tasks(v: &Value) -> Vec<TaskItem> {
    v.as_array()
        .map(|a| a.iter().filter_map(parse_task).collect())
        .unwrap_or_default()
}

/// Task run (`TaskRun`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunItem {
    pub id: i64,
    pub task_id: i64,
    pub started_at: i64,
    /// `running` | `ok` | `error` | `skipped`.
    pub status: String,
    pub output: Option<String>,
}

pub fn parse_run(v: &Value) -> Option<RunItem> {
    Some(RunItem {
        id: v.get("id")?.as_i64()?,
        task_id: v.get("task_id")?.as_i64()?,
        started_at: v.get("started_at")?.as_i64()?,
        status: v.get("status")?.as_str()?.to_string(),
        output: v.get("output").and_then(Value::as_str).map(str::to_string),
    })
}

pub fn parse_runs(v: &Value) -> Vec<RunItem> {
    v.as_array()
        .map(|a| a.iter().filter_map(parse_run).collect())
        .unwrap_or_default()
}
