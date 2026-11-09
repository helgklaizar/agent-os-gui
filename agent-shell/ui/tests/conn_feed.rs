//! Core client events reach `ConnectionController`, banners are set from its view.

use std::time::Instant;

use core_client::{ConnState, CoreEvent};
use ui::conn_feed::ConnBridge;

fn feed(b: &mut ConnBridge, s: ConnState) -> (bool, bool) {
    let v = b.on_event(&CoreEvent::State(s), Instant::now());
    let v = v.unwrap_or_else(|| panic!("state {s:?} did not reach the controller"));
    (v.banner_offline, v.banner_core_unavailable)
}

#[test]
fn conn_bridge_core_unavailable_banner() {
    let mut b = ConnBridge::new();
    assert_eq!(feed(&mut b, ConnState::Connecting), (false, false));
    assert_eq!(feed(&mut b, ConnState::Connected), (false, false));
    assert_eq!(feed(&mut b, ConnState::Reconnecting), (true, false));
    assert_eq!(feed(&mut b, ConnState::CoreUnavailable), (false, true));
    assert_eq!(feed(&mut b, ConnState::Reconnecting), (true, false));
    assert_eq!(feed(&mut b, ConnState::Offline), (true, false));
    assert_eq!(feed(&mut b, ConnState::Connected), (false, false));
    assert!(!b.view().banner_offline);
}

#[test]
fn conn_bridge_ignores_non_state_events() {
    let mut b = ConnBridge::new();
    let ev = CoreEvent::Delivered {
        client_id: "x".into(),
        duplicate: false,
    };
    assert!(b.on_event(&ev, Instant::now()).is_none());
}
