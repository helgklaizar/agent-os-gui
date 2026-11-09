//! S8-T-02/04/07: `IndicatorController` through the public interface and the bus.

mod indicator_common;

use controllers::controllers::indicator::{IndicatorCfg, Msg};
use controllers::indicators::{QuietHours, State};
use indicator_common::Rig;
use std::time::{Duration, Instant};

#[test]
fn frames_color_items_and_unknown_until_first_frame() {
    let mut r = Rig::new();
    r.config(&["cpu", "ram"]);
    assert_eq!(
        r.state_of("cpu"),
        Some(State::Unknown),
        "unknown before the first frame"
    );
    r.frame("cpu", State::Warn, 1);
    assert_eq!(r.state_of("cpu"), Some(State::Warn));
    assert_eq!(r.state_of("ram"), Some(State::Unknown));
}

#[test]
fn hundred_red_frames_raise_one_alert() {
    let mut r = Rig::new();
    r.config(&["disk"]);
    for _ in 0..100 {
        r.frame("disk", State::Bad, 1);
    }
    assert_eq!(r.alerts(), vec![("disk".to_string(), State::Bad)]);
}

#[test]
fn replayed_snapshot_with_old_seq_is_not_an_alert() {
    let mut r = Rig::new();
    r.config(&["disk"]);
    r.frame("disk", State::Bad, 4);
    r.frame("disk", State::Ok, 5);
    r.frame("disk", State::Bad, 4); // snapshot repeated after reconnecting
    assert_eq!(r.alerts(), vec![("disk".to_string(), State::Bad)]);
    assert_eq!(
        r.state_of("disk"),
        Some(State::Ok),
        "a stale frame does not roll the state back"
    );
}

#[test]
fn red_then_yellow_alerts_again_and_return_to_ok_does_not() {
    let mut r = Rig::new();
    r.config(&["disk"]);
    r.frame("disk", State::Bad, 1);
    r.frame("disk", State::Warn, 2);
    r.frame("disk", State::Ok, 3);
    r.frame("disk", State::Bad, 4);
    assert_eq!(
        r.alerts(),
        vec![
            ("disk".to_string(), State::Bad),
            ("disk".to_string(), State::Warn),
            ("disk".to_string(), State::Bad)
        ]
    );
}

#[test]
fn quiet_hours_suppress_yellow_alerts_but_not_red_or_bar_colour() {
    let mut r = Rig::new();
    r.config(&["a", "b", "c"]);
    r.send(Msg::Quiet(Some(QuietHours {
        from: 22 * 60,
        to: 7 * 60,
    })));
    r.send(Msg::Minute(23 * 60));
    r.frame("a", State::Warn, 1);
    r.frame("b", State::Bad, 1);
    assert_eq!(
        r.alerts(),
        vec![("b".to_string(), State::Bad)],
        "yellow is silent, red goes through"
    );
    assert_eq!(
        r.state_of("a"),
        Some(State::Warn),
        "the colour in the bar is not lost"
    );
    r.send(Msg::Minute(8 * 60));
    r.frame("c", State::Warn, 1);
    assert_eq!(r.alerts().len(), 2, "outside quiet hours yellow sounds");
    assert_eq!(r.alerts()[1], ("c".to_string(), State::Warn));
}

#[test]
fn config_order_visibility_and_rebuild_time() {
    let mut r = Rig::new();
    r.send(Msg::Width(2000));
    r.send(Msg::Config(vec![
        IndicatorCfg {
            id: "b".into(),
            enabled: true,
            position: 2,
        },
        IndicatorCfg {
            id: "a".into(),
            enabled: true,
            position: 1,
        },
        IndicatorCfg {
            id: "c".into(),
            enabled: false,
            position: 0,
        },
    ]));
    let ids: Vec<String> = r.view().visible.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        ids,
        vec!["a", "b"],
        "order from Settings, disabled ones hidden"
    );
    let g0 = r.view().generation;

    let big: Vec<IndicatorCfg> = (0..200)
        .map(|i| IndicatorCfg {
            id: format!("i{i}"),
            enabled: true,
            position: (200 - i) as i64,
        })
        .collect();
    let started = Instant::now();
    r.send(Msg::Config(big));
    let took = started.elapsed();
    assert!(took < Duration::from_millis(500), "bar rebuild: {took:?}");
    assert!(
        r.view().generation > g0,
        "a setting change rebuilds the bar"
    );
    assert_eq!(
        r.view().visible.first().map(|i| i.id.clone()),
        Some("i199".to_string())
    );
}

#[test]
fn narrowing_hides_lowest_priority_and_keeps_red() {
    let mut r = Rig::new();
    let ids: Vec<String> = (0..12).map(|i| format!("i{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    r.config(&refs);
    r.frame("i11", State::Bad, 1);
    r.frame("i10", State::Warn, 1);
    r.send(Msg::Width(0));
    let v = r.view();
    assert_eq!(
        v.visible.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["i11"],
        "red is always visible"
    );
    assert_eq!(v.hidden.len(), 11);
    r.send(Msg::Width(2000));
    assert_eq!(r.view().visible.len(), 12);
}

#[test]
fn click_opens_tooltip_and_second_click_or_other_closes_it() {
    let mut r = Rig::new();
    r.config(&["a", "b"]);
    r.send(Msg::Click("a".into()));
    assert!(r.view().visible[0].tooltip_open);
    r.send(Msg::Click("b".into()));
    assert!(!r.view().visible[0].tooltip_open && r.view().visible[1].tooltip_open);
    r.send(Msg::Click("b".into()));
    assert!(r.view().visible.iter().all(|i| !i.tooltip_open));
    r.send(Msg::Click("no-such-id".into()));
    assert!(
        r.view().visible.iter().all(|i| !i.tooltip_open),
        "a foreign id opens nothing"
    );
}

// ---------- S8-T-09: fault tolerance ----------
