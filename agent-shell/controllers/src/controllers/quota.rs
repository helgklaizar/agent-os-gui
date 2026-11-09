//! `QuotaController`: AI account quotas from `bar.quota`.
//!
//! Usage-based accounting: the remaining subscription window is neither shown nor stored; usage, API-key limits
//! and the local model's GPU load are shown. Account names and other core text reach the view verbatim, without markup.

use crate::time::Instant;

use crate::bar_types::{Limit, QuotaItem, QuotaKind};
use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::forecast::{Forecast, duration_text, forecast};
use crate::i18n::{tr, tr_args};
use crate::level::{Level, local_load_level, quota_rate_level, quota_remaining_level, round_pct};
use crate::types::{Fallback, SettingsPage};

#[derive(Debug, Clone, PartialEq)]
pub struct QuotaView {
    pub account_id: i64,
    pub slug: String,
    pub label: String,
    pub provider: String,
    pub kind: QuotaKind,
    /// Remaining, whole percent; `None` means the limit is unknown or it is a local model.
    pub remaining_pct: Option<i64>,
    pub remaining_level: Level,
    /// «1.4×».
    pub rate_text: Option<String>,
    pub rate_level: Level,
    /// GPU load of the local model, percent.
    pub load_pct: Option<i64>,
    pub load_level: Level,
    /// Bar fill 0..1 (remaining; for a local model, load).
    pub bar_fraction: f64,
    /// Overall icon colour: the worse of remaining and rate.
    pub level: Level,
    pub tooltip: String,
    pub accessible_name: String,
    pub no_data: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuotaListView {
    pub items: Vec<QuotaView>,
    pub no_data: bool,
}

// `Delta` carries the whole quota by value: messages are short-lived, boxing would only complicate call sites.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum QuotaMsg {
    /// `None` means a snapshot without data.
    Snapshot(Option<Vec<QuotaItem>>),
    Delta(QuotaItem),
    /// `settings.balance.fallback`; `None` means not known yet.
    Fallback(Option<Fallback>),
    /// Current time (unix seconds) for "resets in" and the forecast.
    Clock(i64),
    Offline,
    Online,
    /// Click on an account's quota.
    Click(i64),
}

#[derive(Debug)]
pub struct QuotaController {
    items: Vec<QuotaItem>,
    no_data: bool,
    online: bool,
    now_unix: i64,
    fallback: Option<Fallback>,
}

impl Default for QuotaController {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            no_data: false,
            online: true,
            now_unix: 0,
            fallback: None,
        }
    }
}

/// Number without trailing zeros: `1.40 → "1.4"`, `5.0 → "5"`, `1.49 → "1.49"`.
fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn limit_args(l: &Limit) -> [(&'static str, String); 2] {
    [
        ("remaining", l.remaining.to_string()),
        ("limit", l.limit.to_string()),
    ]
}

fn limit_line_requests(l: &Limit) -> String {
    let [(a, av), (b, bv)] = limit_args(l);
    tr_args("quota-limit-requests", &[(a, &av), (b, &bv)])
}

fn limit_line_tokens(l: &Limit) -> String {
    let [(a, av), (b, bv)] = limit_args(l);
    tr_args("quota-limit-tokens", &[(a, &av), (b, &bv)])
}

impl QuotaController {
    pub fn new() -> Self {
        Self::default()
    }

    fn view_of(&self, q: &QuotaItem) -> QuotaView {
        let mut v = QuotaView {
            account_id: q.account_id,
            slug: q.slug.clone(),
            label: q.label.clone(),
            provider: q.provider.clone(),
            kind: q.kind,
            remaining_pct: None,
            remaining_level: Level::NoData,
            rate_text: None,
            rate_level: Level::NoData,
            load_pct: None,
            load_level: Level::NoData,
            bar_fraction: 0.0,
            level: Level::NoData,
            tooltip: String::new(),
            accessible_name: String::new(),
            no_data: false,
        };
        if self.no_data || !self.online {
            v.no_data = true;
            v.tooltip = format!("{}\n{}", q.label, tr("bar-no-data"));
            v.accessible_name = tr_args("quota-a11y-nodata", &[("label", &q.label)]);
            return v;
        }
        let mut lines = vec![q.label.clone()];
        match q.kind {
            QuotaKind::Local => self.local(q, &mut v, &mut lines),
            QuotaKind::ApiKey => self.api_key(q, &mut v, &mut lines),
        }
        v.tooltip = lines.join("\n");
        v
    }

    fn local(&self, q: &QuotaItem, v: &mut QuotaView, lines: &mut Vec<String>) {
        lines.push(tr("quota-local"));
        let label = &q.label;
        match q.load {
            Some(load) => {
                let pct = round_pct(load * 100.0);
                v.load_pct = Some(pct);
                v.load_level = local_load_level(load * 100.0);
                v.level = v.load_level;
                v.bar_fraction = load.clamp(0.0, 1.0);
                let pct_text = pct.to_string();
                lines.push(tr_args("quota-local-load", &[("pct", &pct_text)]));
                v.accessible_name =
                    tr_args("quota-a11y-local", &[("label", label), ("pct", &pct_text)]);
            }
            None => {
                v.level = Level::Neutral;
                v.accessible_name = tr_args("quota-a11y-local-nolimit", &[("label", label)]);
            }
        }
    }

    fn api_key(&self, q: &QuotaItem, v: &mut QuotaView, lines: &mut Vec<String>) {
        let Some(used) = q.used else {
            // limit not known yet: usage only, no remaining amounts or forecasts
            v.level = Level::Neutral;
            self.usage(q, lines);
            lines.push(tr("quota-limit-unknown"));
            v.accessible_name = tr_args("quota-a11y-unknown", &[("label", &q.label)]);
            return;
        };
        let remaining = ((1.0 - used) * 100.0).clamp(0.0, 100.0);
        let pct = round_pct(remaining);
        v.remaining_pct = Some(pct);
        v.remaining_level = quota_remaining_level(remaining);
        v.bar_fraction = remaining / 100.0;
        v.level = v.remaining_level;
        if let Some(hours) = q.window_h {
            lines.push(tr_args("quota-window", &[("hours", &trimmed(hours))]));
        }
        lines.push(tr_args("quota-remaining", &[("pct", &pct.to_string())]));
        if let Some(rate) = q.rate {
            let text = trimmed(rate);
            lines.push(tr_args("quota-rate", &[("rate", &text)]));
            v.rate_level = quota_rate_level(rate);
            v.level = v.level.worst(v.rate_level);
            v.rate_text = Some(tr_args("quota-rate-text", &[("rate", &text)]));
        }
        if let Some(reset) = q.reset_at.filter(|r| *r > self.now_unix) {
            let left = duration_text(reset - self.now_unix);
            lines.push(tr_args("quota-reset", &[("duration", &left)]));
        }
        match forecast(self.now_unix, q.used, q.rate, q.window_h, q.reset_at) {
            Forecast::Eta(secs) => {
                lines.push(tr_args("quota-eta", &[("duration", &duration_text(secs))]));
                match self.fallback {
                    Some(Fallback::Local) => lines.push(tr("quota-then-local")),
                    Some(Fallback::Other) => lines.push(tr("quota-then-other")),
                    Some(Fallback::Wait) => lines.push(tr("quota-then-wait")),
                    None => {}
                }
            }
            Forecast::EnoughUntilReset => lines.push(tr("quota-enough")),
            Forecast::Unknown => {}
        }
        self.usage(q, lines);
        if let Some(limits) = &q.limits {
            if let Some(l) = &limits.requests_per_min {
                lines.push(limit_line_requests(l));
            }
            if let Some(l) = &limits.tokens_per_min {
                lines.push(limit_line_tokens(l));
            }
        }
        v.accessible_name = tr_args(
            "quota-a11y",
            &[("label", &q.label), ("pct", &pct.to_string())],
        );
    }

    fn usage(&self, q: &QuotaItem, lines: &mut Vec<String>) {
        if let Some(u) = &q.usage {
            lines.push(tr_args(
                "quota-usage",
                &[
                    ("tokens", &(u.tokens_in + u.tokens_out).to_string()),
                    ("cost", &format!("{:.2}", u.cost)),
                    ("currency", &u.currency),
                ],
            ));
        }
    }
}

impl Controller for QuotaController {
    type Msg = QuotaMsg;
    type View = QuotaListView;

    fn update(&mut self, msg: QuotaMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            QuotaMsg::Snapshot(Some(items)) => {
                self.items = items;
                self.no_data = false;
            }
            QuotaMsg::Snapshot(None) => self.no_data = true,
            QuotaMsg::Delta(item) => {
                match self
                    .items
                    .iter_mut()
                    .find(|i| i.account_id == item.account_id)
                {
                    Some(slot) => *slot = item,
                    None => self.items.push(item),
                }
                self.no_data = false;
            }
            QuotaMsg::Fallback(fallback) => self.fallback = fallback,
            QuotaMsg::Clock(unix) => self.now_unix = unix,
            QuotaMsg::Offline => self.online = false,
            QuotaMsg::Online => self.online = true,
            QuotaMsg::Click(id) => {
                if self.items.iter().any(|i| i.account_id == id) {
                    return vec![Effect::Publish(BusEvent::OpenSettings(
                        SettingsPage::Models,
                    ))];
                }
            }
        }
        Vec::new()
    }

    fn view(&self) -> QuotaListView {
        QuotaListView {
            items: self.items.iter().map(|q| self.view_of(q)).collect(),
            no_data: self.no_data || !self.online,
        }
    }
}
