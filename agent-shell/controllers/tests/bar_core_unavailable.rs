//! S1-fix Z-1: "Core does not start" is a separate bar state, not "No connection".

use std::time::Instant;

use controllers::controllers::bar::{BarController, BarMsg};
use controllers::timezone::Zone;
use controllers::{ConnState, Controller};

#[test]
fn bar_core_unavailable_is_separate_banner() {
    let mut c = BarController::new(Zone::fixed(0));
    let cases = [
        (ConnState::Connecting, false, false),
        (ConnState::Connected, false, false),
        (ConnState::Reconnecting, true, false),
        (ConnState::Offline, true, false),
        (ConnState::CoreUnavailable, true, true),
        (ConnState::Reconnecting, true, false),
        (ConnState::Connected, false, false),
    ];
    for (state, offline, core) in cases {
        c.update(BarMsg::Connection(state), Instant::now());
        let v = c.view();
        assert_eq!(v.offline, offline, "offline for {state:?}");
        assert_eq!(v.core_unavailable, core, "core_unavailable for {state:?}");
    }
}
