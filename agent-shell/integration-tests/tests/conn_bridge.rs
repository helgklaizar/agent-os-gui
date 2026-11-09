//! A real client against `fake-agentd` in `core_down` mode: the bridge sets "The core does not start".

mod common;

use std::time::Instant;

use common::{client, quiet};
use fake_agentd::CoreMode;
use ui::conn_feed::ConnBridge;

async fn pump(
    rx: &async_channel::Receiver<core_client::CoreEvent>,
    b: &mut ConnBridge,
    what: &str,
    done: impl Fn(&ConnBridge) -> bool,
) {
    let end = Instant::now() + common::WAIT;
    while !done(b) {
        let left = end.saturating_duration_since(Instant::now());
        let Ok(Ok(ev)) = tokio::time::timeout(left, rx.recv()).await else {
            panic!("timed out waiting for: {what}; view: {:?}", b.view());
        };
        b.on_event(&ev, Instant::now());
    }
}

#[tokio::test]
async fn conn_bridge_core_down_then_recovers() {
    let d = quiet().await;
    d.set_mode(CoreMode::Down);
    let c = client(&d);
    let rx = c.events();
    let mut b = ConnBridge::new();
    pump(&rx, &mut b, "the core does not start", |b| {
        b.view().banner_core_unavailable
    })
    .await;
    assert!(!b.view().banner_offline, "this is not 'No connection'");
    d.set_mode(CoreMode::Ok);
    pump(&rx, &mut b, "connection restored", |b| {
        !b.view().banner_core_unavailable && !b.view().banner_offline
    })
    .await;
    c.shutdown();
}
