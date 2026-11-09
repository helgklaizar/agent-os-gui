//! S4-09: the effective permission mode, "a lower level can only tighten" (docs/testing.md section 7.1).

use controllers::permissions::{
    Mode, PermTable, default_mode, effective_for_action, effective_mode, persona_rule_options,
    session_options,
};
use serde_json::json;

use Mode::{Allow, Ask, Deny};

#[test]
fn permissions_tightening_table() {
    // global, persona, session → result; None is "—" or "same as persona"
    let table = [
        (Some(Allow), Some(Ask), None, Ask),
        (Some(Allow), None, Some(Deny), Deny),
        (Some(Deny), Some(Allow), None, Deny),
        (Some(Deny), None, Some(Allow), Deny),
        (Some(Ask), Some(Allow), Some(Ask), Ask),
        (None, Some(Deny), None, Deny),
    ];
    for (g, p, s, want) in table {
        assert_eq!(effective_mode(g, p, s), want, "{g:?}/{p:?}/{s:?}");
    }
}

#[test]
fn permissions_never_weaker_than_any_level() {
    let opts = [None, Some(Allow), Some(Ask), Some(Deny)];
    for g in opts {
        for p in opts {
            for s in opts {
                let got = effective_mode(g, p, s);
                for level in [g, p, s].into_iter().flatten() {
                    assert!(
                        got >= level,
                        "{g:?}/{p:?}/{s:?} → {got:?} weaker than {level:?}"
                    );
                }
                let given: Vec<_> = [g, p, s].into_iter().flatten().collect();
                assert!(
                    given.is_empty() && got == Allow || given.contains(&got),
                    "{g:?}/{p:?}/{s:?} → {got:?}"
                );
            }
        }
    }
}

#[test]
fn permissions_defaults_ask() {
    for a in ["send", "delete", "pay", "config_write"] {
        assert_eq!(default_mode(a), Ask, "{a}");
        assert_eq!(
            effective_for_action(a, None, None, None),
            Ask,
            "{a} without rules"
        );
    }
    for a in ["read", "shell", "browse", "memory_write"] {
        assert_eq!(default_mode(a), Allow, "{a}");
    }
    assert_eq!(
        effective_for_action("send", Some(Allow), None, None),
        Allow,
        "an explicit rule beats the default"
    );
    assert_eq!(
        effective_for_action("send", Some(Allow), None, Some(Ask)),
        Ask
    );
    assert_eq!(effective_for_action("send", None, None, Some(Allow)), Allow);
}

#[test]
fn permissions_ui_no_weaken_option() {
    assert_eq!(
        session_options(Some(Deny), None),
        [Deny],
        "global deny: allow is unavailable"
    );
    assert_eq!(session_options(Some(Allow), Some(Ask)), [Ask, Deny]);
    assert_eq!(session_options(Some(Ask), Some(Deny)), [Deny]);
    assert_eq!(session_options(None, None), [Allow, Ask, Deny]);
    assert_eq!(persona_rule_options(Some(Deny)), [Deny]);
    assert_eq!(persona_rule_options(Some(Ask)), [Ask, Deny]);
    assert_eq!(persona_rule_options(None), [Allow, Ask, Deny]);
}

fn rows() -> serde_json::Value {
    json!([
        { "scope": "global", "scope_id": 0, "action": "send", "mode": "allow" },
        { "scope": "global", "scope_id": 0, "action": "delete", "mode": "deny" },
        { "scope": "persona", "scope_id": 10, "action": "send", "mode": "ask" },
        { "scope": "session", "scope_id": 7, "action": "pay", "mode": "deny" },
        { "scope": "session", "scope_id": 8, "action": "send", "mode": "deny" },
        { "scope": "bogus", "scope_id": 1, "action": "send", "mode": "deny" },
        { "scope": "global", "scope_id": 0, "action": "read", "mode": "maybe" }
    ])
}

#[test]
fn permission_table_from_core_rows() {
    let t = PermTable::from_value(&rows());
    assert_eq!(t.rows.len(), 5, "unknown rows are skipped");
    assert_eq!(t.global("send"), Some(Allow));
    assert_eq!(t.persona(Some(10), "send"), Some(Ask));
    assert_eq!(t.persona(Some(11), "send"), None);
    assert_eq!(
        t.effective("send", Some(10), Some(7)),
        Ask,
        "the persona tightens the global allow"
    );
    assert_eq!(
        t.effective("send", Some(10), Some(8)),
        Deny,
        "the row of session 8 tightens"
    );
    assert_eq!(t.effective("delete", None, Some(7)), Deny);
    assert_eq!(
        t.effective("pay", None, Some(7)),
        Deny,
        "the row of the own session"
    );
    assert_eq!(
        t.effective("pay", None, Some(8)),
        Ask,
        "another session has no effect, the default applies"
    );
    assert_eq!(t.effective("shell", None, None), Allow);
}
