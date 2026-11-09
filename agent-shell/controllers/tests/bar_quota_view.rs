//! S2-05, S2-10: local model, quota view, click, offline, escaping.

mod bar_common;

use bar_common::*;
use controllers::controllers::quota::{QuotaController, QuotaMsg};
use controllers::testkit::evil_strings;
use controllers::{BusEvent, Controller, Effect, SettingsPage};

#[test]
fn bar_hides_subscription_remaining() {
    // a frame without remaining data: the subscription remainder is neither shown nor invented (Q1)
    let v = view_of(vec![unknown_limit("cl3")], None).items.remove(0);
    assert!(
        v.remaining_pct.is_none()
            && !v.tooltip.contains("Remaining")
            && !v.accessible_name.contains("left")
    );
}

#[test]
fn quota_tooltip_has_name_window_remaining_rate_reset_usage_limits() {
    let tip = tooltip(api_key("cl1", "Claude · key-1", 0.62, 1.4, 97), None);
    for want in [
        "Claude · key-1",
        "Limit window: 5 h",
        "Remaining: 38%",
        "Rate: 1.4× of the allowed",
        "Resets in",
        "120000",
        "1.84 USD",
        "Request limit: 1500 of 4000 left",
    ] {
        assert!(tip.contains(want), "tooltip lacks \"{want}\": {tip}");
    }
}

#[test]
fn quota_view_icon_bar_rate_and_accessible_name() {
    let v = view_of(vec![api_key("cl1", "Claude · key-1", 0.62, 1.4, 97)], None)
        .items
        .remove(0);
    assert_eq!(v.rate_text.as_deref(), Some("1.4×"));
    assert!((v.bar_fraction - 0.38).abs() < 1e-9);
    assert_eq!(v.provider, "anthropic");
    assert_eq!(v.accessible_name, "Quota Claude · key-1: 38% left");
}

#[test]
fn quota_click_opens_models() {
    let mut c = QuotaController::new();
    let item = api_key("cl1", "Claude · key-1", 0.62, 1.4, 97);
    let id = item.account_id;
    send(&mut c, QuotaMsg::Snapshot(Some(vec![item])));
    let fx = send(&mut c, QuotaMsg::Click(id));
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::OpenSettings(
            SettingsPage::Models
        ))]
    );
    assert!(
        send(&mut c, QuotaMsg::Click(id + 1000)).is_empty(),
        "a click on an unknown account opens nothing"
    );
}

#[test]
fn quota_delta_replaces_or_adds_and_snapshot_none_is_no_data() {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    let a = api_key("cl1", "Claude · key-1", 0.62, 1.4, 97);
    send(
        &mut c,
        QuotaMsg::Snapshot(Some(vec![a.clone(), local("ol1", 0.2)])),
    );
    let mut changed = a.clone();
    changed.used = Some(0.1);
    send(&mut c, QuotaMsg::Delta(changed));
    let v = c.view();
    assert_eq!(v.items.len(), 2);
    assert_eq!(v.items[0].remaining_pct, Some(90));
    send(&mut c, QuotaMsg::Delta(unknown_limit("cl3")));
    assert_eq!(c.view().items.len(), 3);
    send(&mut c, QuotaMsg::Snapshot(None));
    let v = c.view();
    assert!(
        v.no_data && v.items.iter().all(|i| i.no_data),
        "no data: everything is grey"
    );
    send(&mut c, QuotaMsg::Snapshot(Some(vec![a])));
    assert!(!c.view().no_data);
}

#[test]
fn quota_offline_greys_and_online_restores() {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(
        &mut c,
        QuotaMsg::Snapshot(Some(vec![api_key("cl1", "Claude · key-1", 0.62, 1.4, 97)])),
    );
    send(&mut c, QuotaMsg::Offline);
    let v = c.view();
    assert!(
        v.no_data && v.items[0].no_data && v.items[0].level == controllers::level::Level::NoData
    );
    assert!(
        v.items[0].tooltip.contains("no data"),
        "{}",
        v.items[0].tooltip
    );
    send(&mut c, QuotaMsg::Online);
    assert!(!c.view().no_data);
}

#[test]
fn quota_reset_countdown_follows_clock() {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(
        &mut c,
        QuotaMsg::Snapshot(Some(vec![api_key("cl2", "Claude · key-2", 0.18, 0.4, 180)])),
    );
    assert!(c.view().items[0].tooltip.contains("Resets in 3 h"));
    send(&mut c, QuotaMsg::Clock(NOW + 3600));
    assert!(
        c.view().items[0].tooltip.contains("Resets in 2 h"),
        "{}",
        c.view().items[0].tooltip
    );
    send(&mut c, QuotaMsg::Clock(NOW + 3 * 3600 + 5));
    assert!(
        !c.view().items[0].tooltip.contains("Resets in"),
        "no line after the reset: {}",
        c.view().items[0].tooltip
    );
}

#[test]
fn escape_quota_label() {
    for evil in evil_strings() {
        let v = view_of(vec![api_key("cl1", &evil, 0.62, 1.4, 97)], None)
            .items
            .remove(0);
        assert_eq!(v.label, evil, "label is literal");
        assert!(v.tooltip.contains(&evil), "tooltip is literal");
        assert!(
            v.accessible_name.contains(&evil),
            "accessible name is literal"
        );
    }
}
