//! TOML scenarios: steps on the virtual clock (`fake-agentd/scenarios/*.toml`).

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::FrameType;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    DropWs,
    Restore,
    /// The topic returns "no data" (`null` in the fields).
    NoData(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepKind {
    Emit {
        topic: String,
        kind: FrameType,
        data: Value,
    },
    Action(Action),
    /// An event in the feed (insert into `events` and an `events` topic frame).
    Event(ScenarioEvent),
}

/// "Event" scenario step: `events` fields without the service ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioEvent {
    pub kind: String,
    pub sub: String,
    pub level: Option<String>,
    pub text: String,
    pub source: Option<String>,
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub at_ms: u64,
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("scenario file not read: {0}")]
    Read(String),
    #[error("scenario not parsed: {0}")]
    Parse(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    name: String,
    #[serde(default)]
    step: Vec<RawStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    at_ms: u64,
    emit: Option<RawEmit>,
    action: Option<String>,
    event: Option<RawEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvent {
    kind: String,
    sub: String,
    level: Option<String>,
    text: String,
    source: Option<String>,
    session_id: Option<i64>,
}

fn parse_event(e: RawEvent) -> Result<ScenarioEvent, ScenarioError> {
    if !["ai", "task", "msg"].contains(&e.kind.as_str()) {
        return Err(ScenarioError::Parse(format!(
            "event.kind: ai, task or msg, got `{}`",
            e.kind
        )));
    }
    if let Some(l) = &e.level
        && !["info", "warn", "error", "ok"].contains(&l.as_str())
    {
        return Err(ScenarioError::Parse(format!(
            "event.level: info, warn, error or ok, got `{l}`"
        )));
    }
    if e.sub.is_empty() || e.text.is_empty() {
        return Err(ScenarioError::Parse(
            "event.sub and event.text must not be empty".into(),
        ));
    }
    Ok(ScenarioEvent {
        kind: e.kind,
        sub: e.sub,
        level: e.level,
        text: e.text,
        source: e.source,
        session_id: e.session_id,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEmit {
    topic: String,
    #[serde(rename = "type", default = "default_type")]
    kind: String,
    data: Value,
}

fn default_type() -> String {
    "delta".to_string()
}

fn parse_action(text: &str) -> Result<Action, ScenarioError> {
    match text {
        "drop_ws" => Ok(Action::DropWs),
        "restore" => Ok(Action::Restore),
        other => match other.strip_prefix("no_data:") {
            Some(topic) if !topic.is_empty() => Ok(Action::NoData(topic.to_string())),
            _ => Err(ScenarioError::Parse(format!("unknown action `{other}`"))),
        },
    }
}

fn parse_kind(text: &str) -> Result<FrameType, ScenarioError> {
    match text {
        "snapshot" => Ok(FrameType::Snapshot),
        "delta" => Ok(FrameType::Delta),
        "event" => Ok(FrameType::Event),
        other => Err(ScenarioError::Parse(format!(
            "unknown frame type `{other}`"
        ))),
    }
}

impl Scenario {
    pub fn from_toml(text: &str) -> Result<Scenario, ScenarioError> {
        let raw: RawScenario =
            toml::from_str(text).map_err(|e| ScenarioError::Parse(e.message().to_string()))?;
        let mut steps = Vec::with_capacity(raw.step.len());
        let mut last = 0;
        for s in raw.step {
            if s.at_ms < last {
                return Err(ScenarioError::Parse(
                    "steps are not in ascending at_ms order".into(),
                ));
            }
            last = s.at_ms;
            let kind = match (s.emit, s.action, s.event) {
                (Some(e), None, None) => StepKind::Emit {
                    topic: e.topic,
                    kind: parse_kind(&e.kind)?,
                    data: e.data,
                },
                (None, Some(a), None) => StepKind::Action(parse_action(&a)?),
                (None, None, Some(e)) => StepKind::Event(parse_event(e)?),
                _ => {
                    return Err(ScenarioError::Parse(
                        "a step needs exactly one of emit, action and event".into(),
                    ));
                }
            };
            steps.push(Step {
                at_ms: s.at_ms,
                kind,
            });
        }
        Ok(Scenario {
            name: raw.name,
            steps,
        })
    }

    pub fn from_file(path: &Path) -> Result<Scenario, ScenarioError> {
        let text = std::fs::read_to_string(path).map_err(|e| ScenarioError::Read(e.to_string()))?;
        Scenario::from_toml(&text)
    }
}
