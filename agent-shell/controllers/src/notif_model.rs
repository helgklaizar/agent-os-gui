//! Notification feed model: events, counters, subtype registry, selected filter. No view and no effects.

use std::collections::HashMap;

use crate::notif_types::{EventCounts, EventItem, EventKind, EventsFilter, SubtypeItem};
use crate::notif_view::Selection;
use crate::tree_types::SessionItem;

/// How many latest events the client keeps (the rest are `GET /events?before=` pages).
pub const KEEP: usize = 500;

#[derive(Debug, Default)]
pub struct Model {
    /// Newest first (by `id`).
    pub events: Vec<EventItem>,
    pub counts: EventCounts,
    pub registry: Vec<SubtypeItem>,
    pub selection: Selection,
    pub sessions: HashMap<i64, SessionItem>,
    pub collapsed: bool,
    pub utc_offset: i32,
    /// The open session (from the bus): its events are read immediately and no toast is needed for it.
    pub active: Option<i64>,
}

fn matches(f: &EventsFilter, e: &EventItem) -> bool {
    f.kind.is_none_or(|k| e.kind == k.as_str())
        && f.sub.as_ref().is_none_or(|s| &e.sub == s)
        && f.session_id.is_none_or(|s| e.session_id == Some(s))
}

impl Model {
    pub fn snapshot(&mut self, mut items: Vec<EventItem>, counts: EventCounts) {
        items.sort_by_key(|e| std::cmp::Reverse(e.id));
        items.dedup_by_key(|e| e.id);
        items.truncate(KEEP);
        self.events = items;
        self.counts = counts;
    }

    /// `false`: the event already exists (a repeat after resubscribing); counters are untouched.
    pub fn created(&mut self, e: EventItem) -> bool {
        if self.events.iter().any(|x| x.id == e.id) {
            return false;
        }
        let at = self
            .events
            .iter()
            .position(|x| x.id < e.id)
            .unwrap_or(self.events.len());
        if e.unread {
            self.bump(&e.kind, &e.sub, 1);
        }
        self.events.insert(at, e);
        self.events.truncate(KEEP);
        true
    }

    /// `true`: `ids` contained an event that is not in the feed, so the counters may have diverged from the core.
    pub fn read(&mut self, ids: &[i64]) -> bool {
        let mut unknown = false;
        for id in ids {
            match self.events.iter().position(|e| e.id == *id) {
                Some(i) if self.events[i].unread => {
                    self.events[i].unread = false;
                    let (kind, sub) = (self.events[i].kind.clone(), self.events[i].sub.clone());
                    self.bump(&kind, &sub, -1);
                }
                Some(_) => {}
                None => unknown = true,
            }
        }
        unknown
    }

    /// Marks the filter's events as read. The core applies a kind or subtype filter to all events, so its
    /// counters become zero, while the per-session ones decrease by what is in the feed.
    pub fn mark_filter(&mut self, f: &EventsFilter) {
        let hit: Vec<(String, String)> = self
            .events
            .iter_mut()
            .filter(|e| e.unread && matches(f, e))
            .map(|e| {
                e.unread = false;
                (e.kind.clone(), e.sub.clone())
            })
            .collect();
        for (kind, sub) in hit {
            self.bump(&kind, &sub, -1);
        }
        if f.session_id.is_none() {
            let prefix = f.kind.map(|k| format!("{}.", k.as_str()));
            for (key, n) in self.counts.by_sub.iter_mut() {
                let by_kind = prefix.as_ref().is_none_or(|p| key.starts_with(p));
                let by_sub = f
                    .sub
                    .as_ref()
                    .is_none_or(|s| key.ends_with(&format!(".{s}")));
                if by_kind && by_sub {
                    *n = 0;
                }
            }
            self.recount_kinds();
        }
    }

    /// Adds `delta` to the kind and subtype counters (not below zero) and recomputes the total.
    fn bump(&mut self, kind: &str, sub: &str, delta: i64) {
        let key = format!("{kind}.{sub}");
        let cur = self.counts.by_sub.get(&key).copied().unwrap_or(0);
        self.counts.by_sub.insert(key, (cur + delta).max(0));
        self.recount_kinds();
    }

    /// A kind counter is the sum of its subtypes; the total is the sum of kinds.
    fn recount_kinds(&mut self) {
        let mut by_kind = std::collections::BTreeMap::new();
        for (key, n) in &self.counts.by_sub {
            if let Some((kind, _)) = key.split_once('.') {
                *by_kind.entry(kind.to_string()).or_insert(0) += n;
            }
        }
        self.counts.total = by_kind.values().sum();
        self.counts.by_kind = by_kind;
    }

    /// Whether the feed has unread events of the session.
    pub fn has_unread_of(&self, session: i64) -> bool {
        self.events
            .iter()
            .any(|e| e.unread && e.session_id == Some(session))
    }

    pub fn event(&self, id: i64) -> Option<&EventItem> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn select_all(&mut self) {
        self.selection = Selection::All;
    }

    /// Clicking the active kind again clears it.
    pub fn select_kind(&mut self, kind: EventKind) {
        self.selection = match self.selection {
            Selection::Kind(k) if k == kind => Selection::All,
            _ => Selection::Kind(kind),
        };
    }

    /// Clicking the active subtype again returns the selection to the kind.
    pub fn select_sub(&mut self, kind: EventKind, sub: String) {
        self.selection = match &self.selection {
            Selection::Sub(k, s) if *k == kind && *s == sub => Selection::Kind(kind),
            _ => Selection::Sub(kind, sub),
        };
    }

    pub fn in_selection(&self, e: &EventItem) -> bool {
        match &self.selection {
            Selection::All => true,
            Selection::Kind(k) => e.kind == k.as_str(),
            Selection::Sub(k, s) => e.kind == k.as_str() && &e.sub == s,
        }
    }

    pub fn count_kind(&self, kind: EventKind) -> i64 {
        self.counts.by_kind.get(kind.as_str()).copied().unwrap_or(0)
    }

    pub fn count_sub(&self, kind: EventKind, sub: &str) -> i64 {
        let key = format!("{}.{sub}", kind.as_str());
        self.counts.by_sub.get(&key).copied().unwrap_or(0)
    }
}
