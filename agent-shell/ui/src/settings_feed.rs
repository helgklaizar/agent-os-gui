//! Bridge from core-client events to `SettingsController`: `bar.quota` and `bar.sys` (the same frames as the bar, so the
//! local model and hardware on the "Models" page match the bar) and `settings` (the page was changed by another client).
//! No GTK: works on macOS too, so it is covered by integration tests.

use controllers::bar_types::{QuotaItem, parse_quota_item, parse_quota_snapshot, parse_sys};
use controllers::settings_view::SettingsMsg;
use core_client::CoreEvent;
use core_client::api::FrameKind;

use crate::settings_exec::changed;

/// Accumulated quota list: a `bar.quota` delta is one entry, the dialog receives the whole list.
#[derive(Debug, Default)]
pub struct SettingsFeed {
    quotas: Vec<QuotaItem>,
}

impl SettingsFeed {
    pub fn new() -> SettingsFeed {
        SettingsFeed::default()
    }

    /// Core client event -> messages to the dialog. A frame of the wrong shape is skipped (api.md section 2).
    pub fn route(&mut self, event: &CoreEvent) -> Vec<SettingsMsg> {
        let CoreEvent::Frame(f) = event else {
            return Vec::new();
        };
        match (f.topic.as_str(), f.kind) {
            ("bar.quota", FrameKind::Snapshot) => match parse_quota_snapshot(&f.data) {
                Some(items) => {
                    self.quotas = items;
                    vec![SettingsMsg::Quotas(self.quotas.clone())]
                }
                None => Vec::new(),
            },
            ("bar.quota", FrameKind::Delta) => match parse_quota_item(&f.data) {
                Some(item) => {
                    match self
                        .quotas
                        .iter_mut()
                        .find(|q| q.account_id == item.account_id)
                    {
                        Some(slot) => *slot = item,
                        None => self.quotas.push(item),
                    }
                    vec![SettingsMsg::Quotas(self.quotas.clone())]
                }
                None => Vec::new(),
            },
            ("bar.sys", FrameKind::Snapshot | FrameKind::Delta) => {
                vec![SettingsMsg::Sys(parse_sys(&f.data))]
            }
            ("settings", _) => changed(&f.data)
                .map(|(page, rev)| SettingsMsg::Changed { page, rev })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }
}
