//! Tests of `ConnectionController` (S1-07: `connection_state_machine`, `connection_offline_banner_state`).

use std::time::{Duration, Instant};

use controllers::controllers::connection::{
    BACKOFF_MAX, ConnectionController, ConnectionMsg, backoff,
};
use controllers::{BusEvent, ConnState, Controller, Effect};

fn published(fx: &[Effect]) -> Vec<ConnState> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Publish(BusEvent::ConnectionChanged(s)) => Some(*s),
            _ => None,
        })
        .collect()
}

#[test]
fn connection_backoff_grows_to_30s() {
    let secs: Vec<u64> = (1..=8).map(|a| backoff(a).as_secs()).collect();
    assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(backoff(0), Duration::from_secs(1));
    assert_eq!(backoff(u32::MAX), BACKOFF_MAX);
    assert_eq!(BACKOFF_MAX, Duration::from_secs(30));
}

#[test]
fn connection_state_machine() {
    let t0 = Instant::now();
    let mut c = ConnectionController::new();
    assert_eq!(c.view().state, ConnState::Connecting);

    let fx = c.update(ConnectionMsg::Connected, t0);
    assert_eq!(c.view().state, ConnState::Connected);
    assert_eq!(published(&fx), vec![ConnState::Connected]);

    // connection lost: Reconnecting, first attempt after 1 s
    let t1 = t0 + Duration::from_secs(10);
    let fx = c.update(ConnectionMsg::Dropped, t1);
    assert_eq!(c.view().state, ConnState::Reconnecting);
    assert_eq!(c.view().attempt, 1);
    assert_eq!(c.view().retry_at, Some(t1 + Duration::from_secs(1)));
    assert_eq!(published(&fx), vec![ConnState::Reconnecting]);

    // pauses grow: 2, 4, 8, 16, 30, 30
    let mut now = t1;
    for (i, want) in [2u64, 4, 8, 16, 30, 30].into_iter().enumerate() {
        now += Duration::from_secs(1);
        let fx = c.update(ConnectionMsg::AttemptFailed, now);
        assert_eq!(c.view().attempt, i as u32 + 2);
        assert_eq!(
            c.view().retry_at,
            Some(now + Duration::from_secs(want)),
            "step {i}"
        );
        assert!(
            published(&fx).is_empty(),
            "a repeated Reconnecting is not published"
        );
        assert_eq!(c.view().state, ConnState::Reconnecting);
    }

    // success: the counter and the pause are reset
    let fx = c.update(ConnectionMsg::Connected, now);
    assert_eq!(c.view().state, ConnState::Connected);
    assert_eq!(c.view().attempt, 0);
    assert_eq!(c.view().retry_at, None);
    assert_eq!(published(&fx), vec![ConnState::Connected]);

    // the next loss starts from 1 s again
    c.update(ConnectionMsg::Dropped, now);
    assert_eq!(c.view().retry_at, Some(now + Duration::from_secs(1)));
}

#[test]
fn connection_core_unavailable_is_separate_state() {
    let t = Instant::now();
    let mut c = ConnectionController::new();
    let fx = c.update(ConnectionMsg::CoreUnavailable, t);
    assert_eq!(c.view().state, ConnState::CoreUnavailable);
    assert_eq!(published(&fx), vec![ConnState::CoreUnavailable]);
    // and leaves it with a normal connection
    c.update(ConnectionMsg::Connected, t);
    assert_eq!(c.view().state, ConnState::Connected);
}

#[test]
fn connection_offline_banner_state() {
    let t = Instant::now();
    let mut c = ConnectionController::new();
    assert!(!c.view().banner_offline && !c.view().banner_core_unavailable);

    c.update(ConnectionMsg::Connected, t);
    assert!(!c.view().banner_offline);

    c.update(ConnectionMsg::Offline, t);
    assert_eq!(c.view().state, ConnState::Offline);
    assert!(c.view().banner_offline);
    assert!(!c.view().banner_core_unavailable);

    c.update(ConnectionMsg::CoreUnavailable, t);
    assert!(c.view().banner_core_unavailable);
    assert!(
        !c.view().banner_offline,
        "the banners are mutually exclusive"
    );

    c.update(ConnectionMsg::Connected, t);
    c.update(ConnectionMsg::Dropped, t);
    assert!(
        c.view().banner_offline,
        "Reconnecting also shows \"No connection\""
    );
}

#[test]
fn connection_first_connect_failure_goes_reconnecting() {
    let t = Instant::now();
    let mut c = ConnectionController::new();
    c.update(ConnectionMsg::AttemptFailed, t);
    assert_eq!(c.view().state, ConnState::Reconnecting);
    assert_eq!(c.view().attempt, 1);
}

#[test]
fn connection_duplicate_state_is_not_republished() {
    let t = Instant::now();
    let mut c = ConnectionController::new();
    c.update(ConnectionMsg::Connected, t);
    assert!(c.update(ConnectionMsg::Connected, t).is_empty());
    c.update(ConnectionMsg::Offline, t);
    assert!(c.update(ConnectionMsg::Offline, t).is_empty());
}

#[test]
fn connection_stale_attempt_failed_while_connected_is_ignored() {
    let t = Instant::now();
    let mut c = ConnectionController::new();
    c.update(ConnectionMsg::Connected, t);
    let fx = c.update(ConnectionMsg::AttemptFailed, t);
    assert!(fx.is_empty());
    assert_eq!(c.view().state, ConnState::Connected);
}
