//! S2-03…S2-06: routing of `bar.*` frames and wiring of the three bar controllers through `BarCluster`.

mod bar_common;

use bar_common::*;
use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::controllers::metrics::MetricKind;
use controllers::level::Level;
use controllers::timezone::Zone;
use controllers::{BusEvent, ConnState, Effect, Fallback, SettingsPage};
use serde_json::json;

fn cluster() -> BarCluster {
    BarCluster::new(Zone::fixed(7200))
}

fn sys(disk: i64) -> serde_json::Value {
    json!({ "cpu": 14, "gpu": 20, "ram": 66, "disk": disk, "temp": 38 })
}

fn quota_snapshot() -> serde_json::Value {
    json!({ "items": [
        { "account_id": 1, "slug": "cl1", "provider": "anthropic", "label": "Claude · key-1", "kind": "api_key", "window_h": 5.0, "used": 0.62, "rate": 1.4, "reset_at": NOW + 97 * 60, "eta_at": NOW + 4885, "load": null, "usage": { "period": "day", "tokens_in": 120000, "tokens_out": 0, "cost": 1.84, "currency": "USD" }, "limits": null, "updated_at": NOW },
        { "account_id": 3, "slug": "ol1", "provider": "ollama", "label": "Ollama · local", "kind": "local", "window_h": 0.0, "used": null, "rate": null, "reset_at": null, "load": 0.22, "usage": null, "limits": null, "updated_at": NOW },
    ] })
}

#[test]
fn cluster_routes_sys_quota_weather() {
    let mut c = cluster();
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys(38), NOW, t0());
    c.on_frame("bar.sys", FrameKind::Delta, &sys(41), NOW, t0());
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        t0(),
    );
    c.on_frame("bar.weather", FrameKind::Snapshot, &json!({"temp_c": 24, "icon": "sun", "text": "clear", "place": "Jerusalem", "updated_at": NOW}), NOW, t0());
    let v = c.view();
    let disk = v
        .metrics
        .items
        .iter()
        .find(|m| m.kind == MetricKind::Disk)
        .map(|m| m.value);
    assert_eq!(disk, Some(Some(41)), "the delta replaced the value");
    assert_eq!(v.quotas.items.len(), 2);
    assert_eq!(
        (
            v.quotas.items[0].remaining_pct,
            v.quotas.items[0].remaining_level
        ),
        (Some(38), Level::Warn)
    );
    assert!(
        v.quotas.items[0].tooltip.contains("Runs out in 1 h 21 min"),
        "{}",
        v.quotas.items[0].tooltip
    );
    assert_eq!(v.quotas.items[1].load_pct, Some(22));
    assert_eq!(v.bar.weather.temp, "24°");
}

#[test]
fn cluster_quota_delta_updates_one_item() {
    let mut c = cluster();
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        t0(),
    );
    let mut one = quota_snapshot()["items"][0].clone();
    one["used"] = json!(0.9);
    c.on_frame("bar.quota", FrameKind::Delta, &one, NOW, t0());
    let v = c.view();
    assert_eq!(v.quotas.items.len(), 2);
    assert_eq!(v.quotas.items[0].remaining_pct, Some(10));
}

#[test]
fn cluster_garbage_frames_become_no_data_not_panic() {
    let mut c = cluster();
    c.on_frame("bar.sys", FrameKind::Snapshot, &json!("garbage"), NOW, t0());
    assert!(c.view().metrics.no_data);
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &json!({"items": "none"}),
        NOW,
        t0(),
    );
    assert!(c.view().quotas.no_data);
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &json!({"items": null}),
        NOW,
        t0(),
    );
    assert!(c.view().quotas.no_data);
    c.on_frame("bar.weather", FrameKind::Snapshot, &json!(42), NOW, t0());
    assert_eq!(c.view().bar.weather.level, Level::NoData);
    assert!(
        c.on_frame("bar.unknown", FrameKind::Snapshot, &json!({}), NOW, t0())
            .is_empty(),
        "a foreign topic is ignored"
    );
    c.on_frame(
        "bar.quota",
        FrameKind::Delta,
        &json!("not a quota"),
        NOW,
        t0(),
    );
}

#[test]
fn cluster_event_frames_do_not_change_state() {
    let mut c = cluster();
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys(38), NOW, t0());
    let before = c.view();
    c.on_frame("bar.sys", FrameKind::Event, &sys(99), NOW, t0());
    assert_eq!(c.view(), before);
}

#[test]
fn cluster_connection_loss_greys_everything_then_recovers() {
    let mut c = cluster();
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys(38), NOW, t0());
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        t0(),
    );
    c.on_connection(ConnState::Connected, t0());
    assert!(!c.view().bar.offline);
    c.on_connection(ConnState::Reconnecting, t0());
    c.tick(NOW, t0());
    let v = c.view();
    assert!(
        v.bar.offline && v.metrics.no_data && v.quotas.no_data,
        "disconnect: banner, metrics and quotas are grey"
    );
    assert_eq!(
        v.bar.time.len(),
        8,
        "the clock keeps running without a connection"
    );
    c.on_connection(ConnState::Connected, t0());
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys(40), NOW, t0());
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        t0(),
    );
    let v = c.view();
    assert!(!v.bar.offline && !v.metrics.no_data && !v.quotas.no_data);
}

#[test]
fn cluster_tick_drives_clock_quota_countdown_and_stale_metrics() {
    let mut c = cluster();
    let start = t0();
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys(38), NOW, start);
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        start,
    );
    c.tick(NOW, start);
    assert_eq!(c.view().bar.time, "16:05:09");
    c.tick(NOW + 1, later(start, 1_000));
    assert_eq!(c.view().bar.time, "16:05:10");
    assert!(!c.view().metrics.no_data);
    c.tick(NOW + 7, later(start, 7_000));
    assert!(c.view().metrics.no_data, "metrics are stale");
    assert!(
        c.view().quotas.items[0]
            .tooltip
            .contains("Resets in 1 h 36 min"),
        "{}",
        c.view().quotas.items[0].tooltip
    );
}

#[test]
fn cluster_fallback_and_click() {
    let mut c = cluster();
    c.on_frame(
        "bar.quota",
        FrameKind::Snapshot,
        &quota_snapshot(),
        NOW,
        t0(),
    );
    c.tick(NOW, t0());
    c.set_fallback(Some(Fallback::Other), t0());
    assert!(
        c.view().quotas.items[0]
            .tooltip
            .contains("Then: another key")
    );
    let id = c.view().quotas.items[0].account_id;
    let fx = c.click_quota(id, t0());
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::OpenSettings(
            SettingsPage::Models
        ))]
    );
}
