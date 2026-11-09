//! Event subtype registry (`docs/reference/api.md` 5.3): core data, not a schema enum. The GUI does not enumerate providers.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subtype {
    pub kind: &'static str,
    pub sub: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
}

const fn s(
    kind: &'static str,
    sub: &'static str,
    label: &'static str,
    icon: &'static str,
) -> Subtype {
    Subtype {
        kind,
        sub,
        label,
        icon,
    }
}

/// Eleven default subtypes. The `msg.wa` label carries the mark of an unofficial channel (Whapi).
pub const SUBTYPES: [Subtype; 11] = [
    s("ai", "agent", "Agent", "agent"),
    s("ai", "watch", "Observer", "eye"),
    s("ai", "claude", "Claude", "claude"),
    s("ai", "local", "Local model", "local"),
    s("task", "sched", "Scheduled", "clock"),
    s("task", "deleg", "Delegated", "delegate"),
    s("task", "done", "Done", "check"),
    s("msg", "sys", "System", "system"),
    s("msg", "wa", "WhatsApp (unofficial channel)", "whatsapp"),
    s("msg", "tg", "Telegram", "telegram"),
    s("msg", "mail", "Mail", "mail"),
];

/// Response of `GET /events/subtypes`.
#[must_use]
pub fn subtypes_json() -> Value {
    Value::Array(
        SUBTYPES
            .iter()
            .map(|t| json!({"kind": t.kind, "sub": t.sub, "label": t.label, "icon": t.icon, "enabled": true}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_unique_pairs() {
        let mut pairs: Vec<_> = SUBTYPES.iter().map(|t| (t.kind, t.sub)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), 11);
        assert!(
            SUBTYPES
                .iter()
                .all(|t| !t.label.is_empty() && !t.icon.is_empty())
        );
    }
}
