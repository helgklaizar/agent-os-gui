//! Shared helpers for notification feed tests: events, registry, a controller with an initial snapshot.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::Instant;

use controllers::controllers::notifications::{NotifMsg, NotificationsController};
use controllers::notif_types::{EventCounts, EventItem, EventKind, SubtypeItem};
use controllers::notif_view::{EventCard, KindChip, NotifView, SubChip};
use controllers::{Controller, Effect};

/// The 11 default registry subtypes: (kind, subtype, label, icon).
pub const REGISTRY: [(&str, &str, &str, &str); 11] = [
    ("ai", "agent", "Agent", "agent-icon"),
    ("ai", "watch", "Watcher", "watch-icon"),
    ("ai", "claude", "Claude", "claude-icon"),
    ("ai", "local", "Local", "local-icon"),
    ("task", "sched", "Scheduled", "sched-icon"),
    ("task", "deleg", "Delegated", "deleg-icon"),
    ("task", "done", "Completed", "done-icon"),
    ("msg", "sys", "System", "sys-icon"),
    ("msg", "wa", "WhatsApp", "wa-icon"),
    ("msg", "tg", "Telegram", "tg-icon"),
    ("msg", "mail", "Mail", "mail-icon"),
];

pub fn registry() -> Vec<SubtypeItem> {
    REGISTRY
        .iter()
        .map(|(kind, sub, label, icon)| SubtypeItem {
            kind: (*kind).to_string(),
            sub: (*sub).to_string(),
            label: (*label).to_string(),
            icon: Some((*icon).to_string()),
            enabled: true,
        })
        .collect()
}

pub fn ev(
    id: i64,
    kind: &str,
    sub: &str,
    text: &str,
    session: Option<i64>,
    unread: bool,
) -> EventItem {
    EventItem {
        id,
        kind: kind.to_string(),
        sub: sub.to_string(),
        level: "info".to_string(),
        text: text.to_string(),
        session_id: session,
        tab: "chat".to_string(),
        source: None,
        unread,
        ts: 1_793_571_261 + id,
    }
}

/// Unread counters as the core counts them.
pub fn counts_of(items: &[EventItem]) -> EventCounts {
    let mut c = EventCounts::default();
    for e in items.iter().filter(|e| e.unread) {
        c.total += 1;
        *c.by_kind.entry(e.kind.clone()).or_insert(0) += 1;
        *c.by_sub.entry(format!("{}.{}", e.kind, e.sub)).or_insert(0) += 1;
    }
    c
}

pub fn no_counts() -> EventCounts {
    EventCounts {
        total: 0,
        by_kind: BTreeMap::new(),
        by_sub: BTreeMap::new(),
    }
}

pub struct Rig {
    pub c: NotificationsController,
    pub now: Instant,
}

impl Rig {
    pub fn send(&mut self, msg: NotifMsg) -> Vec<Effect> {
        self.c.update(msg, self.now)
    }

    pub fn view(&self) -> NotifView {
        self.c.view()
    }

    pub fn card_ids(&self) -> Vec<i64> {
        self.view().cards.iter().map(|c| c.id).collect()
    }

    pub fn card(&self, id: i64) -> Option<EventCard> {
        self.view().cards.into_iter().find(|c| c.id == id)
    }

    pub fn kind(&self, kind: EventKind) -> KindChip {
        match self.view().kinds.into_iter().find(|k| k.kind == kind) {
            Some(k) => k,
            None => panic!("no filter for kind {kind:?}"),
        }
    }

    pub fn sub(&self, kind: EventKind, sub: &str) -> Option<SubChip> {
        self.kind(kind).subs.into_iter().find(|s| s.sub == sub)
    }
}

/// A controller with a registry and an `items` snapshot (the counters are computed from them).
pub fn rig(items: Vec<EventItem>) -> Rig {
    let mut r = Rig {
        c: NotificationsController::new(),
        now: Instant::now(),
    };
    r.send(NotifMsg::Subtypes(registry()));
    let counts = counts_of(&items);
    r.send(NotifMsg::Snapshot { items, counts });
    r
}

pub fn effects_of_commands(fx: &[Effect]) -> Vec<controllers::CoreCommand> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Command(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

pub fn published(fx: &[Effect]) -> Vec<controllers::BusEvent> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Publish(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}
