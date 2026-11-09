//! `IndicatorController`: additional bar indicators from topic `bar.indicators`.
//!
//! The core keeps the state and the transition counter; the controller shows the state, collapses indicators by priority,
//! shows silent periodic sources in grey and raises [`BusEvent::IndicatorAlert`] once per transition
//! (yellow ones are suppressed during quiet hours, red ones always go through). The feed event is created by the core, not the GUI.

use std::collections::HashMap;
use std::time::Duration;

use crate::time::Instant;

use serde_json::Value;

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::indicators::{BarItem, QuietHours, State, capacity_for_width, collapse, manifest};

/// A source with no frames for longer than this is considered failed: the indicator turns grey ("after 10 s").
const STALE_FLOOR: Duration = Duration::from_secs(10);

/// Length limit of `text` in a frame: tooltip details are short, a longer one is a source error.
pub const MAX_TEXT_CHARS: usize = 500;

/// Indicator setting from the `indicators` table (enabled flag and order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorCfg {
    pub id: String,
    pub enabled: bool,
    pub position: i64,
}

/// A `bar.indicators` frame (`IndicatorState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub id: String,
    pub state: State,
    /// The core's `transition_seq`; `0` means the source sends no counter.
    pub seq: u64,
    pub value: Option<f64>,
    /// Details for the tooltip (`IndicatorState.text`); the view escapes them itself.
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("indicator frame not parsed: {0}")]
    Invalid(String),
}

/// Parses a `bar.indicators` frame; an unreadable frame is an error, not a silent skip.
pub fn parse_frame(v: &Value) -> Result<Frame, FrameError> {
    let bad = |m: &str| FrameError::Invalid(m.to_string());
    let obj = v.as_object().ok_or_else(|| bad("object expected"))?;
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("id"))?;
    let state = obj
        .get("state")
        .and_then(Value::as_str)
        .and_then(State::parse)
        .ok_or_else(|| bad("state"))?;
    let seq = match obj.get("seq") {
        None | Some(Value::Null) => 0,
        Some(s) => s.as_u64().ok_or_else(|| bad("seq"))?,
    };
    let value = match obj.get("value") {
        None | Some(Value::Null) => None,
        Some(x) => Some(x.as_f64().ok_or_else(|| bad("value"))?),
    };
    let text = match obj.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.chars().count() <= MAX_TEXT_CHARS => Some(t.clone()),
        Some(_) => return Err(bad("text")),
    };
    Ok(Frame {
        id: id.to_string(),
        state,
        seq,
        value,
        text,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// Setting from `indicators`: rebuilds the bar.
    Config(Vec<IndicatorCfg>),
    Frame(Frame),
    /// The source reported a failure: the indicator turns grey immediately.
    SourceFailed(String),
    /// Bar width, px.
    Width(u32),
    /// Current minute of the day (local time) for quiet hours.
    Minute(u16),
    Quiet(Option<QuietHours>),
    Click(String),
    /// Check for silent sources; frames and other messages also advance time.
    Tick,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub id: String,
    pub state: State,
    pub tooltip_open: bool,
    /// Last value of the source; `None` means a frame without a value or a failed source.
    pub value: Option<f64>,
    /// Tooltip details from the last frame.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarView {
    pub visible: Vec<ItemView>,
    pub hidden: Vec<String>,
    /// Grows on every rebuild caused by a setting or the width.
    pub generation: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    state: State,
    seq: u64,
    last_frame: Option<Instant>,
    value: Option<f64>,
    text: Option<String>,
}

#[derive(Debug)]
pub struct IndicatorController {
    cfg: Vec<IndicatorCfg>,
    entries: HashMap<String, Entry>,
    capacity: usize,
    quiet: Option<QuietHours>,
    minute: u16,
    open_tooltip: Option<String>,
    generation: u64,
    now: Option<Instant>,
}

impl Default for IndicatorController {
    fn default() -> Self {
        IndicatorController {
            cfg: Vec::new(),
            entries: HashMap::new(),
            // width not known yet: hide nothing
            capacity: usize::MAX,
            quiet: None,
            minute: 0,
            open_tooltip: None,
            generation: 0,
            now: None,
        }
    }
}

impl IndicatorController {
    fn stale_after(id: &str) -> Option<Duration> {
        let interval = manifest(id)?.interval_ms?;
        Some(STALE_FLOOR.max(Duration::from_millis(interval.saturating_mul(2))))
    }

    /// State for display: a silent periodic source means "no data".
    fn shown(&self, id: &str) -> State {
        let Some(e) = self.entries.get(id) else {
            return State::Unknown;
        };
        let silent = match (self.now, e.last_frame, Self::stale_after(id)) {
            (Some(now), Some(last), Some(limit)) => now.saturating_duration_since(last) > limit,
            _ => false,
        };
        if silent { State::NoData } else { e.state }
    }

    /// Value and details for display: a grey ("no data") indicator does not show the old ones.
    fn shown_detail(&self, id: &str) -> (Option<f64>, Option<String>) {
        match self.entries.get(id) {
            Some(e) if self.shown(id) != State::NoData => (e.value, e.text.clone()),
            _ => (None, None),
        }
    }

    fn quiet_now(&self) -> bool {
        self.quiet.is_some_and(|q| q.contains(self.minute))
    }

    fn on_frame(&mut self, f: Frame, now: Instant) -> Vec<Effect> {
        let entry = self.entries.entry(f.id.clone()).or_insert(Entry {
            state: State::Unknown,
            seq: 0,
            last_frame: None,
            value: None,
            text: None,
        });
        if f.seq != 0 && f.seq < entry.seq {
            return Vec::new(); // repeated snapshot after reconnecting
        }
        let newer = f.seq == 0 || f.seq > entry.seq;
        let changed = f.state != entry.state;
        entry.state = f.state;
        entry.seq = entry.seq.max(f.seq);
        entry.last_frame = Some(now);
        entry.value = f.value;
        entry.text = f.text;
        let alertable = matches!(f.state, State::Warn | State::Bad);
        if changed && newer && alertable && !(f.state == State::Warn && self.quiet_now()) {
            return vec![Effect::Publish(BusEvent::IndicatorAlert {
                id: f.id,
                state: f.state,
            })];
        }
        Vec::new()
    }
}

impl Controller for IndicatorController {
    type Msg = Msg;
    type View = BarView;

    fn update(&mut self, msg: Msg, now: Instant) -> Vec<Effect> {
        self.now = Some(self.now.map_or(now, |p| p.max(now)));
        match msg {
            Msg::Config(cfg) => {
                self.cfg = cfg;
                self.generation += 1;
                Vec::new()
            }
            Msg::Frame(f) => self.on_frame(f, now),
            Msg::SourceFailed(id) => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.state = State::NoData;
                }
                Vec::new()
            }
            Msg::Width(px) => {
                let capacity = capacity_for_width(px);
                if capacity != self.capacity {
                    self.capacity = capacity;
                    self.generation += 1;
                }
                Vec::new()
            }
            Msg::Minute(m) => {
                self.minute = m;
                Vec::new()
            }
            Msg::Quiet(q) => {
                self.quiet = q;
                Vec::new()
            }
            Msg::Click(id) => {
                let known = self.cfg.iter().any(|c| c.enabled && c.id == id);
                self.open_tooltip = match &self.open_tooltip {
                    Some(open) if *open == id => None,
                    _ if known => Some(id),
                    _ => None,
                };
                Vec::new()
            }
            Msg::Tick => Vec::new(),
        }
    }

    fn view(&self) -> BarView {
        let items: Vec<BarItem> = self
            .cfg
            .iter()
            .map(|c| BarItem {
                id: c.id.clone(),
                state: self.shown(&c.id),
                position: c.position,
                enabled: c.enabled,
            })
            .collect();
        let layout = collapse(&items, self.capacity);
        let visible = layout
            .visible
            .into_iter()
            .map(|id| {
                let (value, detail) = self.shown_detail(&id);
                ItemView {
                    state: self.shown(&id),
                    tooltip_open: self.open_tooltip.as_deref() == Some(id.as_str()),
                    value,
                    detail,
                    id,
                }
            })
            .collect();
        BarView {
            visible,
            hidden: layout.hidden,
            generation: self.generation,
        }
    }
}
