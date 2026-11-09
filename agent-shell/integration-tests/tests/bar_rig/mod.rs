//! Shared stand of the bar tests against `fake-agentd`: `Rig`, waiting for frames, reading `balance.fallback`.
#![allow(dead_code)]

use std::time::Instant;

use crate::common::{WAIT, client, http, start};
use controllers::bar_cluster::BarCluster;
use controllers::controllers::metrics::MetricKind;
use controllers::level::Level;
use controllers::timezone::Zone;
use core_client::{CoreClient, CoreEvent, HttpClient};
use fake_agentd::{Config, FakeAgentd};
use ui::bar_feed::{TOPICS, apply_fallback, route};

pub struct Rig {
    pub d: FakeAgentd,
    pub c: CoreClient,
    pub http: HttpClient,
    pub cluster: BarCluster,
    pub rx: async_channel::Receiver<CoreEvent>,
}

pub async fn rig(config: Config) -> Rig {
    let d = start(config).await;
    let c = client(&d);
    let rx = c.events();
    for t in TOPICS {
        assert!(c.subscribe(t).is_ok());
    }
    let http = http(&d, Some(d.admin_token()));
    Rig {
        d,
        c,
        http,
        cluster: BarCluster::new(Zone::fixed(0)),
        rx,
    }
}

impl Rig {
    /// Parses events until the condition holds; returns whether `fallback` had to be re-read.
    pub async fn pump(&mut self, what: &str, done: impl Fn(&BarCluster) -> bool) -> bool {
        let mut refresh = false;
        let end = Instant::now() + WAIT;
        while !done(&self.cluster) {
            let left = end.saturating_duration_since(Instant::now());
            let ev = tokio::time::timeout(left, self.rx.recv()).await;
            let Ok(Ok(ev)) = ev else {
                panic!(
                    "timed out waiting for: {what}; view: {:?}",
                    self.cluster.view()
                )
            };
            let out = route(&mut self.cluster, &ev, self.d.now_unix(), Instant::now());
            refresh |= out.refresh_fallback;
        }
        refresh
    }

    /// Parses events until the "re-read `fallback`" flag arrives.
    pub async fn pump_until_refresh(&mut self) -> bool {
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(ev)) = tokio::time::timeout(left, self.rx.recv()).await else {
                return false;
            };
            if route(&mut self.cluster, &ev, self.d.now_unix(), Instant::now()).refresh_fallback {
                return true;
            }
        }
        false
    }

    pub async fn read_fallback(&mut self) {
        let page = self.http.personas().await;
        let value = page.ok().and_then(|p| p.balance).and_then(|b| b.fallback);
        apply_fallback(&mut self.cluster, value.as_deref(), Instant::now());
    }
}

pub fn disk(c: &BarCluster) -> Option<i64> {
    c.view()
        .metrics
        .items
        .iter()
        .find(|m| m.kind == MetricKind::Disk)
        .and_then(|m| m.value)
}

pub fn loaded(c: &BarCluster) -> bool {
    let v = c.view();
    v.quotas.items.len() == 3
        && !v.quotas.no_data
        && !v.metrics.no_data
        && v.bar.weather.level != Level::NoData
}
