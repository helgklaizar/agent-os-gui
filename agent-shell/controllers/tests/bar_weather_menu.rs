//! S2-06, S2-07, S2-10: weather from `bar.weather`, the Agent OS menu, Logout.

mod bar_common;

use bar_common::*;
use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::bar_types::WeatherFrame;
use controllers::controllers::bar::{BarController, BarMsg};
use controllers::level::Level;
use controllers::testkit::evil_strings;
use controllers::timezone::Zone;
use controllers::{
    BusEvent, ConnState, Controller, DialogKind, Effect, FocusTarget, MenuItem, SettingsPage,
};
use serde_json::json;

fn weather(temp: f64, icon: &str) -> WeatherFrame {
    WeatherFrame {
        temp_c: Some(temp),
        icon: Some(icon.to_string()),
        text: Some("clear".to_string()),
        place: Some("Jerusalem".to_string()),
        updated_at: Some(NOW),
    }
}

#[test]
fn weather_topic_bar_weather() {
    let mut cluster = BarCluster::new(Zone::fixed(0));
    let data = json!({ "temp_c": 24, "icon": "sun", "text": "clear", "place": "Jerusalem", "updated_at": NOW });
    cluster.on_frame("bar.weather", FrameKind::Snapshot, &data, NOW, t0());
    let w = cluster.view().bar.weather;
    assert_eq!(
        (w.temp.as_str(), w.icon, w.level),
        ("24°", "weather-clear-symbolic", Level::Neutral)
    );
    assert!(
        w.tooltip.contains("clear") && w.tooltip.contains("Jerusalem"),
        "{}",
        w.tooltip
    );
    assert_eq!((w.text.as_str(), w.place.as_str()), ("clear", "Jerusalem"));
}

#[test]
fn weather_change_of_frame_changes_value_and_icon() {
    let mut c = BarController::new(Zone::fixed(0));
    send(&mut c, BarMsg::Weather(Some(weather(21.0, "sun"))));
    assert_eq!(
        (c.view().weather.temp.clone(), c.view().weather.icon),
        ("21°".to_string(), "weather-clear-symbolic")
    );
    send(&mut c, BarMsg::Weather(Some(weather(14.0, "rain"))));
    assert_eq!(
        (c.view().weather.temp.clone(), c.view().weather.icon),
        ("14°".to_string(), "weather-showers-symbolic")
    );
    send(&mut c, BarMsg::Weather(Some(weather(-3.0, "snow"))));
    assert_eq!(c.view().weather.temp, "-3°");
    assert_eq!(c.view().weather.icon, "weather-snow-symbolic");
    for (name, icon) in [
        ("moon", "weather-clear-night-symbolic"),
        ("partly", "weather-few-clouds-symbolic"),
        ("cloud", "weather-overcast-symbolic"),
        ("storm", "weather-storm-symbolic"),
        ("fog", "weather-fog-symbolic"),
        ("blizzard-of-x", "weather-none-available-symbolic"),
    ] {
        send(&mut c, BarMsg::Weather(Some(weather(5.0, name))));
        assert_eq!(c.view().weather.icon, icon, "{name}");
    }
}

#[test]
fn weather_no_data_dash() {
    let mut c = BarController::new(Zone::fixed(0));
    let w = c.view().weather;
    assert_eq!(
        (w.temp.as_str(), w.level),
        ("—", Level::NoData),
        "no frame yet: a grey \"—\", not a hard-coded 21°C"
    );
    send(&mut c, BarMsg::Weather(Some(weather(21.0, "sun"))));
    send(&mut c, BarMsg::Weather(None));
    let w = c.view().weather;
    assert_eq!(
        (w.temp.as_str(), w.level, w.icon),
        ("—", Level::NoData, "weather-none-available-symbolic")
    );
    let all_null = WeatherFrame {
        text: Some("x".into()),
        ..WeatherFrame::default()
    };
    send(&mut c, BarMsg::Weather(Some(all_null)));
    assert_eq!(
        c.view().weather.level,
        Level::NoData,
        "no temperature means no data"
    );
}

#[test]
fn weather_source_down_leaves_rest_of_bar() {
    let mut cluster = BarCluster::new(Zone::fixed(0));
    cluster.on_frame(
        "bar.sys",
        FrameKind::Snapshot,
        &json!({"cpu": 14, "gpu": 20, "ram": 66, "disk": 38, "temp": 38}),
        NOW,
        t0(),
    );
    cluster.tick(NOW, t0());
    let before = cluster.view();
    cluster.on_frame(
        "bar.weather",
        FrameKind::Delta,
        &json!({"temp_c": null, "icon": null, "text": null, "place": null, "updated_at": null}),
        NOW,
        t0(),
    );
    let after = cluster.view();
    assert_eq!(after.bar.weather.level, Level::NoData);
    assert_eq!(
        (
            after.metrics,
            after.quotas,
            after.bar.time,
            after.bar.offline
        ),
        (
            before.metrics,
            before.quotas,
            before.bar.time,
            before.bar.offline
        )
    );
}

#[test]
fn escape_weather_fields() {
    for evil in evil_strings() {
        let mut c = BarController::new(Zone::fixed(0));
        let mut w = weather(20.0, "sun");
        w.text = Some(evil.clone());
        w.place = Some(evil.clone());
        send(&mut c, BarMsg::Weather(Some(w)));
        let v = c.view().weather;
        assert_eq!(
            (v.text.as_str(), v.place.as_str()),
            (evil.as_str(), evil.as_str())
        );
        assert!(v.tooltip.contains(&evil));
    }
}

#[test]
fn menu_items_events() {
    let mut c = BarController::new(Zone::fixed(0));
    let labels: Vec<_> = c
        .view()
        .menu
        .iter()
        .map(|e| (e.item, e.label.clone()))
        .collect();
    assert_eq!(
        labels,
        [
            (MenuItem::About, "About".into()),
            (MenuItem::User, "User".into()),
            (MenuItem::Settings, "Settings".into()),
            (MenuItem::Logout, "Logout".into())
        ]
    );
    let cases = [
        (
            MenuItem::About,
            BusEvent::OpenDialog {
                kind: DialogKind::About,
                opener: FocusTarget::MenuItem(MenuItem::About),
            },
        ),
        (
            MenuItem::User,
            BusEvent::OpenSettings(SettingsPage::Account),
        ),
        (
            MenuItem::Settings,
            BusEvent::OpenSettings(SettingsPage::Root),
        ),
        (
            MenuItem::Logout,
            BusEvent::OpenDialog {
                kind: DialogKind::ConfirmLogout,
                opener: FocusTarget::MenuItem(MenuItem::Logout),
            },
        ),
    ];
    for (item, event) in cases {
        send(&mut c, BarMsg::MenuToggle);
        assert!(c.view().menu_open);
        let fx = send(&mut c, BarMsg::MenuSelect(item));
        assert_eq!(fx, vec![Effect::Publish(event)], "{item:?}");
        assert!(
            !c.view().menu_open,
            "the menu is closed after a selection: {item:?}"
        );
    }
}

#[test]
fn menu_dismiss_by_outside_click_or_esc_closes_without_events() {
    let mut c = BarController::new(Zone::fixed(0));
    assert!(send(&mut c, BarMsg::MenuToggle).is_empty());
    assert!(c.view().menu_open);
    assert!(send(&mut c, BarMsg::MenuDismiss).is_empty());
    assert!(!c.view().menu_open);
    assert!(
        send(&mut c, BarMsg::MenuDismiss).is_empty(),
        "a closed menu has nothing to close"
    );
    send(&mut c, BarMsg::MenuToggle);
    send(&mut c, BarMsg::MenuToggle);
    assert!(
        !c.view().menu_open,
        "a second click on the button closes it"
    );
    assert!(
        send(&mut c, BarMsg::MenuSelect(MenuItem::About)).len() == 1,
        "a selection without opening still yields the event"
    );
}

#[test]
fn bar_offline_banner_follows_connection() {
    let mut c = BarController::new(Zone::fixed(0));
    for (state, offline) in [
        (ConnState::Connecting, false),
        (ConnState::Connected, false),
        (ConnState::Reconnecting, true),
        (ConnState::Offline, true),
        (ConnState::CoreUnavailable, true),
        (ConnState::Connected, false),
    ] {
        send(&mut c, BarMsg::Connection(state));
        assert_eq!(c.view().offline, offline, "{state:?}");
    }
}
