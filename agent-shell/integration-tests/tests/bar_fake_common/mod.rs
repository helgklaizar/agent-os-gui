//! Shared helper of the bar frame tests: the theme snapshot from `fake-agentd`.
#![allow(dead_code)]

use crate::common::next_frame;
use core_client::CoreClient;
use core_client::api::Frame;

pub async fn snapshot_of(c: &CoreClient, topic: &str) -> Frame {
    let rx = c.events();
    assert!(c.subscribe(topic).is_ok());
    match next_frame(&rx, topic).await {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    }
}
