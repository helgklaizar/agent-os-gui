//! S2-05: the "Runs out in" forecast and the "Then" text in the quota tooltip.

mod bar_common;

use bar_common::*;
use controllers::bar_types::QuotaKind;
use controllers::controllers::quota::{QuotaController, QuotaMsg};
use controllers::forecast::{Forecast, duration_text, forecast};
use controllers::{Controller, Fallback};

#[test]
fn quota_forecast_shown_when_eta_before_reset() {
    // used 0.62, rate 1.4, window 5 h: ETA = 0.38 / (1.4 / 5) h ≈ 81 min, reset in 97 min
    let tip = tooltip(api_key("cl1", "Claude · key-1", 0.62, 1.4, 97), None);
    assert!(tip.contains("Runs out in 1 h 21 min"), "{tip}");
    assert!(!tip.contains("Enough until reset"), "{tip}");
    assert!(tip.contains("Resets in 1 h 37 min"), "{tip}");
}

#[test]
fn quota_forecast_hidden_when_eta_after_reset() {
    // prototype regression: same values, reset in 60 min
    let tip = tooltip(api_key("cl1", "Claude · key-1", 0.62, 1.4, 60), None);
    assert!(tip.contains("Enough until reset"), "{tip}");
    assert!(!tip.contains("Runs out"), "{tip}");
}

#[test]
fn quota_forecast_hidden_when_rate_le_1() {
    let tip = tooltip(api_key("cl2", "Claude · key-2", 0.18, 0.4, 180), None);
    assert!(tip.contains("Enough until reset"), "{tip}");
    assert!(!tip.contains("Runs out"), "{tip}");
    let at_one = tooltip(api_key("cl2", "Claude · key-2", 0.62, 1.0, 600), None);
    assert!(
        at_one.contains("Enough until reset"),
        "rate = 1 does not burn faster than allowed: {at_one}"
    );
}

#[test]
fn quota_forecast_boundary_eta_equals_reset() {
    // ETA exactly at the reset moment: "before the reset" does not hold
    let eta_secs = ((1.0_f64 - 0.5) / (2.0 / 4.0) * 3600.0) as i64; // 1 h
    assert_eq!(
        forecast(NOW, Some(0.5), Some(2.0), Some(4.0), Some(NOW + eta_secs)),
        Forecast::EnoughUntilReset
    );
    assert_eq!(
        forecast(
            NOW,
            Some(0.5),
            Some(2.0),
            Some(4.0),
            Some(NOW + eta_secs + 1)
        ),
        Forecast::Eta(eta_secs)
    );
    assert_eq!(
        forecast(NOW, None, Some(2.0), Some(4.0), Some(NOW + 9999)),
        Forecast::Unknown
    );
    assert_eq!(
        forecast(NOW, Some(0.5), Some(2.0), Some(4.0), None),
        Forecast::Unknown
    );
    assert_eq!(
        forecast(NOW, Some(0.5), Some(2.0), Some(0.0), Some(NOW + 9999)),
        Forecast::Unknown
    );
}

#[test]
fn quota_duration_text() {
    for (secs, want) in [
        (4885, "1 h 21 min"),
        (7200, "2 h"),
        (2700, "45 min"),
        (59, "less than a minute"),
        (0, "less than a minute"),
        (3600, "1 h"),
    ] {
        assert_eq!(duration_text(secs), want, "{secs} s");
    }
}

#[test]
fn quota_fallback_text_follows_setting() {
    let item = api_key("cl1", "Claude · key-1", 0.62, 1.4, 97);
    for (fb, want) in [
        (Fallback::Local, "Then: local model"),
        (Fallback::Other, "Then: another key"),
        (Fallback::Wait, "Then: wait for reset"),
    ] {
        let tip = tooltip(item.clone(), Some(fb));
        assert!(tip.contains(want), "{fb:?}: {tip}");
        assert_eq!(tip.matches("Then:").count(), 1, "{tip}");
    }
    assert!(
        !tooltip(item.clone(), None).contains("Then"),
        "setting unknown: no such line"
    );
    // "Then" appears only together with a forecast
    let enough = tooltip(
        api_key("cl1", "Claude · key-1", 0.62, 1.4, 60),
        Some(Fallback::Wait),
    );
    assert!(!enough.contains("Then"), "{enough}");
}

#[test]
fn quota_fallback_change_updates_tooltip() {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(
        &mut c,
        QuotaMsg::Snapshot(Some(vec![api_key("cl1", "Claude · key-1", 0.62, 1.4, 97)])),
    );
    send(&mut c, QuotaMsg::Fallback(Some(Fallback::Local)));
    assert!(c.view().items[0].tooltip.contains("local model"));
    send(&mut c, QuotaMsg::Fallback(Some(Fallback::Wait)));
    assert!(c.view().items[0].tooltip.contains("wait for reset"));
}

#[test]
fn quota_local_model_no_limit() {
    let v = view_of(vec![local("ol1", 0.22)], Some(Fallback::Local))
        .items
        .remove(0);
    assert!(v.tooltip.contains("no limits"), "{}", v.tooltip);
    assert!(v.tooltip.contains("22%"), "{}", v.tooltip);
    assert!(
        !v.tooltip.contains("Runs out")
            && !v.tooltip.contains("Enough")
            && !v.tooltip.contains("Then"),
        "{}",
        v.tooltip
    );
    assert_eq!(
        (v.kind, v.remaining_pct, v.rate_text.clone()),
        (QuotaKind::Local, None, None)
    );
    assert_eq!(v.load_pct, Some(22));
    assert!((v.bar_fraction - 0.22).abs() < 1e-9);
}

#[test]
fn quota_unknown_limit_shows_usage_only() {
    let v = view_of(vec![unknown_limit("cl3")], Some(Fallback::Local))
        .items
        .remove(0);
    assert_eq!((v.remaining_pct, v.rate_text.clone()), (None, None));
    assert!(v.tooltip.contains("Limit unknown"), "{}", v.tooltip);
    assert!(v.tooltip.contains("900"), "usage is visible: {}", v.tooltip);
    for absent in ["Remaining", "Runs out", "Enough", "Resets", "Then", "Rate"] {
        assert!(!v.tooltip.contains(absent), "{absent}: {}", v.tooltip);
    }
    assert!(
        v.accessible_name.contains("limit unknown"),
        "{}",
        v.accessible_name
    );
}
