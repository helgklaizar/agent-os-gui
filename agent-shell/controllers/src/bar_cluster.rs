//! The whole bar: `BarController`, `MetricsController` and `QuotaController` together. `bar.*` frames and connection events are routed here
//! through a single entry point; the controllers themselves do not know about each other (`controllers_do_not_import_each_other`).

use crate::time::Instant;

use serde_json::Value;

use crate::bar_types::{parse_quota_item, parse_quota_snapshot, parse_sys, parse_weather};
use crate::controller::{Controller, Effect};
use crate::controllers::bar::{BarController, BarMsg, BarView};
use crate::controllers::metrics::{MetricsController, MetricsMsg, MetricsView};
use crate::controllers::quota::{QuotaController, QuotaListView, QuotaMsg};
use crate::timezone::Zone;
use crate::types::{ConnState, Fallback};

/// WS frame type (docs/reference/api.md, section 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Snapshot,
    Delta,
    Event,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BarClusterView {
    pub bar: BarView,
    pub metrics: MetricsView,
    pub quotas: QuotaListView,
}

pub struct BarCluster {
    pub bar: BarController,
    pub metrics: MetricsController,
    pub quota: QuotaController,
}

impl BarCluster {
    pub fn new(zone: Zone) -> Self {
        Self {
            bar: BarController::new(zone),
            metrics: MetricsController::new(),
            quota: QuotaController::new(),
        }
    }

    /// A frame of topic `bar.sys`, `bar.quota` or `bar.weather`; other topics and events (`event`) are ignored.
    /// A frame that cannot be parsed is shown as "no data" rather than silently dropped.
    pub fn on_frame(
        &mut self,
        topic: &str,
        kind: FrameKind,
        data: &Value,
        unix: i64,
        now: Instant,
    ) -> Vec<Effect> {
        if kind == FrameKind::Event {
            return Vec::new();
        }
        match topic {
            "bar.sys" => self.metrics.update(MetricsMsg::Frame(parse_sys(data)), now),
            "bar.quota" => {
                self.quota.update(QuotaMsg::Clock(unix), now);
                let msg = match kind {
                    FrameKind::Delta => {
                        parse_quota_item(data).map_or(QuotaMsg::Snapshot(None), QuotaMsg::Delta)
                    }
                    _ => QuotaMsg::Snapshot(parse_quota_snapshot(data)),
                };
                self.quota.update(msg, now)
            }
            "bar.weather" => self
                .bar
                .update(BarMsg::Weather(Some(parse_weather(data))), now),
            _ => Vec::new(),
        }
    }

    pub fn on_connection(&mut self, state: ConnState, now: Instant) -> Vec<Effect> {
        let mut fx = self.bar.update(BarMsg::Connection(state), now);
        let (metrics, quota) = match state {
            ConnState::Connected => (MetricsMsg::Online, QuotaMsg::Online),
            ConnState::Connecting => return fx,
            _ => (MetricsMsg::Offline, QuotaMsg::Offline),
        };
        fx.extend(self.metrics.update(metrics, now));
        fx.extend(self.quota.update(quota, now));
        fx
    }

    pub fn set_fallback(&mut self, fallback: Option<Fallback>, now: Instant) -> Vec<Effect> {
        self.quota.update(QuotaMsg::Fallback(fallback), now)
    }

    /// Click on an account's quota icon.
    pub fn click_quota(&mut self, account_id: i64, now: Instant) -> Vec<Effect> {
        self.quota.update(QuotaMsg::Click(account_id), now)
    }

    /// Once per second: clock, "resets in", stale-metrics check.
    pub fn tick(&mut self, unix: i64, now: Instant) -> Vec<Effect> {
        let mut fx = self.bar.update(BarMsg::Tick(unix), now);
        fx.extend(self.quota.update(QuotaMsg::Clock(unix), now));
        fx.extend(self.metrics.update(MetricsMsg::Tick, now));
        fx
    }

    pub fn view(&self) -> BarClusterView {
        BarClusterView {
            bar: self.bar.view(),
            metrics: self.metrics.view(),
            quotas: self.quota.view(),
        }
    }
}
