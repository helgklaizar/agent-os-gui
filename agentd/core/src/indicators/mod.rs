//! Bar indicators: states, sources, the transition store port and the polling engine.
//! Live values are not written to the DB — only state transitions and the events they produce.

mod engine;
mod sources;
mod sys;
mod system;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{FieldError, PortFuture};

pub use engine::IndicatorEngine;
pub use sources::{AgentdStatus, AiUsage, ObserverHealth, Unread};
pub use sys::SysIndicator;
pub use system::{
    BACKUP_BAD_SECS, BACKUP_WARN_SECS, Link, SystemIndicator, SystemProbe, grade_backup,
    grade_battery, grade_net, grade_vpn,
};

/// Indicators enabled by default: CPU, RAM, disk, temperature, network, AI usage, unread, `agentd`.
pub const DEFAULT_INDICATORS: [&str; 8] = [
    "cpu", "ram", "disk", "temp", "net", "ai_usage", "unread", "agentd",
];
pub const MAX_ID_CHARS: usize = 64;

/// Default document of the `indicators` page.
#[must_use]
pub fn default_items() -> Value {
    let items: Vec<Value> = DEFAULT_INDICATORS
        .iter()
        .enumerate()
        .map(|(i, id)| json!({"id": id, "enabled": true, "position": i}))
        .collect();
    Value::Array(items)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorState {
    Ok,
    Warn,
    Bad,
    Nodata,
    Unknown,
}

impl IndicatorState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            IndicatorState::Ok => "ok",
            IndicatorState::Warn => "warn",
            IndicatorState::Bad => "bad",
            IndicatorState::Nodata => "nodata",
            IndicatorState::Unknown => "unknown",
        }
    }

    /// Only values from the `CHECK` of the `indicators` table; case matters.
    #[must_use]
    pub fn parse(s: &str) -> Option<IndicatorState> {
        match s {
            "ok" => Some(IndicatorState::Ok),
            "warn" => Some(IndicatorState::Warn),
            "bad" => Some(IndicatorState::Bad),
            "nodata" => Some(IndicatorState::Nodata),
            "unknown" => Some(IndicatorState::Unknown),
            _ => None,
        }
    }
}

/// One source frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub state: IndicatorState,
    pub value: Option<Value>,
    pub text: Option<String>,
}

impl Reading {
    #[must_use]
    pub fn new(state: IndicatorState) -> Reading {
        Reading {
            state,
            value: None,
            text: None,
        }
    }
}

/// Core metric source; system sources plug in through the same interface.
pub trait IndicatorSource: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn read(&self) -> PortFuture<'_, Reading>;
}

#[derive(Debug, PartialEq)]
pub enum IndicatorError {
    Invalid(Vec<FieldError>),
    Unavailable,
}

/// Frame result: `frame` — `IndicatorState {id, state, value, text, ts}`; `transitioned` — the state changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub frame: Value,
    pub transitioned: bool,
}

/// Transition store. State change, the `transition_seq` counter and the event are one transaction; the frame goes to
/// the `bar.indicators` topic after commit, the event to the `events` topic.
pub trait IndicatorStore: Send + Sync + 'static {
    fn observe(
        &self,
        id: String,
        reading: Reading,
    ) -> PortFuture<'_, Result<Observed, IndicatorError>>;
    /// `snapshot` of the `bar.indicators` topic: `{items: [IndicatorState]}` for indicators that have a state.
    fn snapshot(&self) -> PortFuture<'_, Result<Value, IndicatorError>>;
}

/// Indicator identifier: `a-z0-9_.-`, up to 64 characters (`quota.cl1`, `whapi`).
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_CHARS
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        })
}

pub(crate) type Sources = Vec<Arc<dyn IndicatorSource>>;
