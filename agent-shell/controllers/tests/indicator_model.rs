//! S8-T-01 (thresholds), S8-T-02 (collapsing), S8-01 (manifest): the indicator model without GTK.

use controllers::ConnState;
use controllers::indicators::{
    BarItem, ClickAction, DEFAULT_ENABLED, Group, QuietHours, Rule, State, agentd_state,
    capacity_for_width, catalog, collapse, manifest, state_for,
};

use State::{Bad, NoData, Ok as Good, Warn};

/// `(indicator, value, expected state)`: the boundaries and their neighbourhood per the thresholds table.
const TABLE: &[(&str, f64, State)] = &[
    // battery, %: ≤30 yellow, ≤15 red
    ("battery", 31.0, Good),
    ("battery", 30.0, Warn),
    ("battery", 16.0, Warn),
    ("battery", 15.0, Bad),
    ("battery", 0.0, Bad),
    // load average / core count: > 1 yellow, > 2 red
    ("load", 1.0, Good),
    ("load", 1.01, Warn),
    ("load", 2.0, Warn),
    ("load", 2.01, Bad),
    // remaining AI limit, %: ≤40 / ≤15
    ("ai.usage", 41.0, Good),
    ("ai.usage", 40.0, Warn),
    ("ai.usage", 16.0, Warn),
    ("ai.usage", 15.0, Bad),
    // agent queue: N − maxParallel, red when > 0
    ("ai.queue", -3.0, Good),
    ("ai.queue", 0.0, Good),
    ("ai.queue", 1.0, Bad),
    // VRAM, %: ≥80 / ≥95
    ("ai.vram", 79.0, Good),
    ("ai.vram", 80.0, Warn),
    ("ai.vram", 94.0, Warn),
    ("ai.vram", 95.0, Bad),
    // next meeting, minutes: < 10 yellow
    ("msg.meeting", 11.0, Good),
    ("msg.meeting", 10.0, Good),
    ("msg.meeting", 9.0, Warn),
    // backup, hours: > 26 / > 50
    ("infra.backup", 26.0, Good),
    ("infra.backup", 27.0, Warn),
    ("infra.backup", 50.0, Warn),
    ("infra.backup", 51.0, Bad),
    // certificates, days: ≤14 / ≤5 or expired
    ("infra.cert", 15.0, Good),
    ("infra.cert", 14.0, Warn),
    ("infra.cert", 6.0, Warn),
    ("infra.cert", 5.0, Bad),
    ("infra.cert", 0.0, Bad),
    ("infra.cert", -1.0, Bad),
    // cron per day: ≥1 red
    ("infra.cron", 0.0, Good),
    ("infra.cron", 1.0, Bad),
    // server disk (module), %: ≥80 / ≥85
    ("srv.disk", 79.0, Good),
    ("srv.disk", 80.0, Warn),
    ("srv.disk", 84.0, Warn),
    ("srv.disk", 85.0, Bad),
    // flags: 0 is normal, 1 is an event
    ("net.link", 1.0, Good),
    ("net.link", 0.0, Bad),
    ("audio.mic", 1.0, Warn),
    ("audio.mic", 0.0, Good),
    ("sys.units", 0.0, Good),
    ("sys.units", 1.0, Bad),
    ("sys.updates", 1.0, Warn),
    ("ai.watch", 1.0, Bad),
    ("ai.watch", 0.0, Good),
    ("srv.container", 1.0, Bad),
];

#[test]
fn thresholds_on_and_around_boundaries() {
    for &(id, value, want) in TABLE {
        let m = manifest(id).unwrap_or_else(|| panic!("no indicator {id} in the catalog"));
        assert_eq!(state_for(m, Some(value)), want, "{id} = {value}");
    }
}

#[test]
fn no_value_means_nodata_for_every_indicator() {
    assert!(!catalog().is_empty(), "the indicator catalog is empty");
    for m in catalog() {
        assert_eq!(state_for(m, None), NoData, "{}: no value: grey", m.id);
        assert_eq!(state_for(m, Some(f64::NAN)), NoData, "{}: NaN: grey", m.id);
    }
}

#[test]
fn display_only_indicators_are_ok_with_a_value() {
    for id in [
        "net.rate",
        "ai.cost",
        "ai.tokens",
        "msg.unread",
        "bluetooth",
        "layout",
    ] {
        let m = manifest(id).unwrap_or_else(|| panic!("no indicator {id} in the catalog"));
        assert_eq!(
            m.rule,
            Rule::None,
            "{id}: thresholds are not given by the table"
        );
        assert_eq!(state_for(m, Some(3.0)), Good, "{id}");
    }
}

#[test]
fn catalog_covers_four_groups_with_unique_ids_and_tooltip_click() {
    let mut ids: Vec<&str> = catalog().iter().map(|m| m.id).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "indicator ids are unique");
    for g in [
        Group::System,
        Group::Ai,
        Group::Messages,
        Group::Infra,
        Group::Module,
    ] {
        assert!(
            catalog().iter().any(|m| m.group == g),
            "no indicators of group {g:?}"
        );
    }
    assert!(
        catalog()
            .iter()
            .all(|m| m.click_action == ClickAction::Tooltip && !m.source.is_empty())
    );
}

#[test]
fn default_set_is_q2_and_everything_else_is_off() {
    assert_eq!(
        DEFAULT_ENABLED,
        [
            "cpu",
            "ram",
            "disk",
            "temp",
            "net.link",
            "ai.usage",
            "msg.unread",
            "infra.agentd"
        ],
        "the default set (Q2)"
    );
}

#[test]
fn state_strings_follow_d38() {
    for (s, text) in [
        (Good, "ok"),
        (Warn, "warn"),
        (Bad, "bad"),
        (NoData, "nodata"),
        (State::Unknown, "unknown"),
    ] {
        assert_eq!(s.as_str(), text);
        assert_eq!(State::parse(text), Some(s));
    }
    assert_eq!(State::parse("red"), None);
}

fn item(id: &str, state: State, position: i64) -> BarItem {
    BarItem {
        id: id.into(),
        state,
        position,
        enabled: true,
    }
}

#[test]
fn collapse_keeps_all_red_even_without_room() {
    let items = [item("a", Bad, 0), item("b", Bad, 1), item("c", Good, 2)];
    let c = collapse(&items, 0);
    assert_eq!(c.visible, vec!["a", "b"]);
    assert_eq!(c.hidden, vec!["c"]);
}

#[test]
fn collapse_drops_lowest_priority_first_and_shows_in_settings_order() {
    let items = [
        item("a", Good, 0),
        item("b", Warn, 1),
        item("c", Bad, 2),
        item("d", Good, 3),
        item("e", Warn, 4),
    ];
    let c = collapse(&items, 3);
    assert_eq!(
        c.visible,
        vec!["b", "c", "e"],
        "red, then yellow; order as in Settings"
    );
    assert_eq!(c.hidden, vec!["a", "d"]);
    let c = collapse(&items, 4);
    assert_eq!(
        c.visible,
        vec!["a", "b", "c", "e"],
        "the top one of the rest by order came back next"
    );
    assert_eq!(c.hidden, vec!["d"]);
    let c = collapse(&items, 5);
    assert_eq!(c.visible, vec!["a", "b", "c", "d", "e"]);
    assert!(c.hidden.is_empty());
}

#[test]
fn collapse_ignores_disabled_items_and_sorts_by_position() {
    let mut off = item("x", Bad, 0);
    off.enabled = false;
    let items = [item("b", Good, 5), off, item("a", Good, 1)];
    let c = collapse(&items, 9);
    assert_eq!(c.visible, vec!["a", "b"]);
    assert!(
        c.hidden.is_empty(),
        "disabled ones do not get into the hidden either"
    );
}

#[test]
fn nodata_ranks_with_the_rest_not_above_yellow() {
    let items = [item("a", NoData, 0), item("b", Warn, 1)];
    assert_eq!(collapse(&items, 1).visible, vec!["b"]);
}

#[test]
fn default_set_fits_1280_and_1024_and_capacity_is_monotone() {
    let n = DEFAULT_ENABLED.len();
    assert!(
        capacity_for_width(1280) >= n,
        "1280 px fits the default set"
    );
    assert!(
        capacity_for_width(1024) >= n,
        "the minimum window of 1024 px fits the default set"
    );
    let mut prev = 0;
    for w in (320..=2560).step_by(64) {
        let c = capacity_for_width(w);
        assert!(c >= prev, "capacity does not decrease with width: {w}");
        prev = c;
    }
    assert!(
        capacity_for_width(640) < capacity_for_width(1280),
        "narrowing leaves less room"
    );
}

#[test]
fn quiet_hours_wrap_over_midnight() {
    let night = QuietHours {
        from: 22 * 60,
        to: 7 * 60,
    };
    assert!(night.contains(23 * 60));
    assert!(night.contains(0));
    assert!(night.contains(6 * 60 + 59));
    assert!(!night.contains(7 * 60));
    assert!(!night.contains(12 * 60));
    assert!(night.contains(22 * 60));
    let day = QuietHours {
        from: 13 * 60,
        to: 14 * 60,
    };
    assert!(day.contains(13 * 60 + 30));
    assert!(!day.contains(14 * 60));
    assert!(!QuietHours { from: 5, to: 5 }.contains(5), "empty interval");
}

#[test]
fn agentd_indicator_follows_connection_state() {
    assert_eq!(agentd_state(ConnState::Connected), Good);
    assert_eq!(agentd_state(ConnState::Connecting), State::Unknown);
    assert_eq!(agentd_state(ConnState::Reconnecting), Warn);
    assert_eq!(agentd_state(ConnState::Offline), Bad);
    assert_eq!(agentd_state(ConnState::CoreUnavailable), Bad);
}
