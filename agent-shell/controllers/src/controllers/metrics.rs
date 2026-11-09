//! `MetricsController`: CPU, GPU, RAM, disk, temperature from `bar.sys`.
//!
//! Live values are kept in memory only (docs/reference/db.md). No frames for more than three periods (6 s) or the connection is lost: everything turns grey.

use crate::time::Instant;
use std::time::Duration;

use crate::bar_types::SysFrame;
use crate::controller::{Controller, Effect};
use crate::i18n::{tr, tr_args};
use crate::level::{Level, load_level, round_pct, temp_level};

/// Three `bar.sys` periods of 2 s each.
pub const STALE_AFTER: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Cpu,
    Gpu,
    Ram,
    Disk,
    Temp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricView {
    pub kind: MetricKind,
    /// Tooltip label ("Disk", "Temperature").
    pub label: String,
    /// Short label in the bar.
    pub caption: String,
    /// «38%», «38°», «—».
    pub text: String,
    pub value: Option<i64>,
    pub level: Level,
    pub accessible_name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricsView {
    pub items: Vec<MetricView>,
    /// No frames for more than three periods (6 s) or no connection: everything is grey.
    pub no_data: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetricsMsg {
    Frame(SysFrame),
    /// Time tick: checks "no frames for more than three periods".
    Tick,
    Offline,
    Online,
}

#[derive(Debug)]
pub struct MetricsController {
    frame: SysFrame,
    last_frame: Option<Instant>,
    stale: bool,
    online: bool,
}

impl Default for MetricsController {
    fn default() -> Self {
        Self {
            frame: SysFrame::default(),
            last_frame: None,
            stale: false,
            online: true,
        }
    }
}

impl MetricsController {
    pub fn new() -> Self {
        Self::default()
    }

    fn no_data(&self) -> bool {
        let empty = [
            self.frame.cpu,
            self.frame.gpu,
            self.frame.ram,
            self.frame.disk,
            self.frame.temp,
        ]
        .iter()
        .all(Option::is_none);
        !self.online || self.stale || self.last_frame.is_none() || empty
    }

    fn item(&self, kind: MetricKind, raw: Option<f64>, grey: bool) -> MetricView {
        let (label, caption) = match kind {
            MetricKind::Cpu => (tr("metric-cpu"), tr("metric-cpu")),
            MetricKind::Gpu => (tr("metric-gpu"), tr("metric-gpu")),
            MetricKind::Ram => (tr("metric-ram"), tr("metric-ram")),
            MetricKind::Disk => (tr("metric-disk"), tr("metric-disk")),
            MetricKind::Temp => (tr("metric-temp"), tr("metric-temp-short")),
        };
        let value = raw.filter(|_| !grey).map(round_pct);
        let (text, level) = match (value, raw) {
            (Some(v), Some(r)) if kind == MetricKind::Temp => (
                tr_args("metric-value-degrees", &[("v", &v.to_string())]),
                temp_level(r),
            ),
            (Some(v), Some(r)) => (
                tr_args("metric-value-percent", &[("v", &v.to_string())]),
                load_level(r),
            ),
            _ => (tr("value-none"), Level::NoData),
        };
        let accessible_name = if value.is_some() {
            tr_args("metric-a11y", &[("label", &label), ("value", &text)])
        } else {
            tr_args("metric-a11y-nodata", &[("label", &label)])
        };
        MetricView {
            kind,
            label,
            caption,
            text,
            value,
            level,
            accessible_name,
        }
    }
}

impl Controller for MetricsController {
    type Msg = MetricsMsg;
    type View = MetricsView;

    fn update(&mut self, msg: MetricsMsg, now: Instant) -> Vec<Effect> {
        match msg {
            MetricsMsg::Frame(frame) => {
                self.frame = frame;
                self.last_frame = Some(now);
                self.stale = false;
            }
            MetricsMsg::Tick => {
                self.stale = self
                    .last_frame
                    .is_none_or(|t| now.saturating_duration_since(t) >= STALE_AFTER);
            }
            MetricsMsg::Offline => self.online = false,
            MetricsMsg::Online => self.online = true,
        }
        Vec::new()
    }

    fn view(&self) -> MetricsView {
        let grey = self.no_data();
        let f = &self.frame;
        MetricsView {
            items: vec![
                self.item(MetricKind::Cpu, f.cpu, grey),
                self.item(MetricKind::Gpu, f.gpu, grey),
                self.item(MetricKind::Ram, f.ram, grey),
                self.item(MetricKind::Disk, f.disk, grey),
                self.item(MetricKind::Temp, f.temp, grey),
            ],
            no_data: grey,
        }
    }
}
