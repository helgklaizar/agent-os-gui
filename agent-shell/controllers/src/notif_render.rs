//! Building the feed view from the model: a filter panel with counters and a list of cards.

use crate::clock_fmt::format_time;
use crate::i18n::{tr, tr_args};
use crate::notif_model::Model;
use crate::notif_types::{EventItem, EventKind};
use crate::notif_view::{
    Accent, AllChip, Badge, CardAction, EventCard, KindChip, NotifView, Selection, StripItem,
    SubChip,
};

pub fn kind_label(kind: EventKind) -> String {
    match kind {
        EventKind::Ai => tr("notifications-kind-ai"),
        EventKind::Task => tr("notifications-kind-task"),
        EventKind::Msg => tr("notifications-kind-msg"),
    }
}

pub fn kind_icon(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Ai => "applications-engineering-symbolic",
        EventKind::Task => "checkbox-checked-symbolic",
        EventKind::Msg => "mail-unread-symbolic",
    }
}

/// Badge: empty at zero, "99+" from a hundred; the screen-reader name is "Label: N unread".
pub fn badge(label: &str, count: i64) -> Badge {
    let text = match count {
        n if n <= 0 => None,
        n if n > 99 => Some("99+".to_string()),
        n => Some(n.to_string()),
    };
    Badge {
        count,
        text,
        a11y_name: tr_args(
            "notifications-unread",
            &[("label", label), ("n", &count.to_string())],
        ),
    }
}

fn kind_chip(m: &Model, kind: EventKind) -> KindChip {
    let label = kind_label(kind);
    let shown = match &m.selection {
        Selection::Kind(k) | Selection::Sub(k, _) => *k == kind,
        Selection::All => false,
    };
    let selected_sub = match &m.selection {
        Selection::Sub(_, s) => Some(s.as_str()),
        _ => None,
    };
    let subs = if shown {
        m.registry
            .iter()
            .filter(|s| s.enabled && s.kind == kind.as_str())
            .map(|s| SubChip {
                sub: s.sub.clone(),
                label: s.label.clone(),
                icon: s.icon.clone(),
                badge: badge(&s.label, m.count_sub(kind, &s.sub)),
                active: selected_sub == Some(s.sub.as_str()),
            })
            .collect()
    } else {
        Vec::new()
    };
    KindChip {
        kind,
        badge: badge(&label, m.count_kind(kind)),
        label,
        icon: kind_icon(kind),
        active: matches!(&m.selection, Selection::Kind(k) | Selection::Sub(k, _) if *k == kind),
        subs,
    }
}

/// Toast label: "session: text", or just the event text for an unknown session.
pub fn toast_text(m: &Model, e: &EventItem) -> String {
    match e.session_id.and_then(|id| session_label(m, id)) {
        Some(name) => tr_args(
            "notifications-toast",
            &[("session", &name), ("text", &e.text)],
        ),
        None => e.text.clone(),
    }
}

/// Session name for the card; for a thread, "parent › thread". If the session is not loaded, there is no label.
fn session_label(m: &Model, id: i64) -> Option<String> {
    let s = m.sessions.get(&id)?;
    match s.parent_id.and_then(|p| m.sessions.get(&p)) {
        Some(parent) => Some(tr_args(
            "notifications-thread-path",
            &[("parent", &parent.name), ("thread", &s.name)],
        )),
        None => Some(s.name.clone()),
    }
}

fn accent(kind: &str) -> Accent {
    match EventKind::parse(kind) {
        Some(EventKind::Task) => Accent::Task,
        Some(EventKind::Msg) => Accent::Msg,
        Some(EventKind::Ai) | None => Accent::Ai,
    }
}

fn card(m: &Model, e: &EventItem) -> EventCard {
    let known = m
        .registry
        .iter()
        .find(|s| s.kind == e.kind && s.sub == e.sub);
    let sub_label = match known {
        Some(s) if !s.label.is_empty() => s.label.clone(),
        _ => e.sub.clone(),
    };
    let (action, action_label) = match e.session_id {
        Some(_) => (CardAction::OpenChat, tr("notifications-open-chat")),
        None => (CardAction::Ok, tr("notifications-ok")),
    };
    EventCard {
        id: e.id,
        kind: e.kind.clone(),
        sub: e.sub.clone(),
        sub_label,
        icon: known.and_then(|s| s.icon.clone()),
        level: e.level.clone(),
        time: format_time(e.ts, m.utc_offset).chars().take(5).collect(),
        session_id: e.session_id,
        session_label: e.session_id.and_then(|id| session_label(m, id)),
        text: e.text.clone(),
        unread: e.unread,
        new_label: e.unread.then(|| tr("notifications-new")),
        accent: accent(&e.kind),
        action,
        action_label,
    }
}

fn strip(m: &Model) -> Vec<StripItem> {
    EventKind::ALL
        .into_iter()
        .map(|kind| {
            let label = kind_label(kind);
            StripItem {
                kind,
                icon: kind_icon(kind),
                badge: badge(&label, m.count_kind(kind)),
                label,
            }
        })
        .collect()
}

pub fn build(m: &Model) -> NotifView {
    let all_label = tr("notifications-all");
    let total = m.counts.total;
    let empty = if !m.events.is_empty() && !m.events.iter().any(|e| m.in_selection(e)) {
        Some(tr("notifications-filter-empty"))
    } else if m.events.is_empty() {
        Some(tr("notifications-empty"))
    } else {
        None
    };
    let title = if total > 0 {
        tr_args("notifications-title-unread", &[("n", &total.to_string())])
    } else {
        tr("app-name")
    };
    NotifView {
        title,
        empty,
        total_unread: total,
        selection: m.selection.clone(),
        all: AllChip {
            badge: badge(&all_label, total),
            label: all_label,
            active: m.selection == Selection::All,
        },
        kinds: EventKind::ALL
            .into_iter()
            .map(|k| kind_chip(m, k))
            .collect(),
        cards: m
            .events
            .iter()
            .filter(|e| m.in_selection(e))
            .map(|e| card(m, e))
            .collect(),
        collapsed: m.collapsed,
        strip: if m.collapsed { strip(m) } else { Vec::new() },
    }
}
