//! WS client for the end-to-end tests, built on `core-client`.

use std::time::{Duration, Instant};

use core_client::api::{Frame, FrameKind};
use core_client::{CoreClient, CoreEvent};
use serde_json::Value;

pub struct Ws {
    pub client: CoreClient,
}

impl Ws {
    pub(super) fn new(client: CoreClient) -> Ws {
        Ws { client }
    }

    pub(super) fn wait(&self, mut pred: impl FnMut(&CoreEvent) -> bool) {
        let rx = self.client.events();
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            match rx.try_recv() {
                Ok(ev) if pred(&ev) => return,
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        panic!("timed out waiting for an event");
    }

    /// Subscribes and waits for `snapshot`; returns its data.
    pub fn subscribe(&mut self, topic: &str) -> Value {
        self.client.subscribe(topic).unwrap();
        let f = self.next_frame(Duration::from_secs(10)).expect("snapshot");
        assert_eq!((f.topic.as_str(), f.kind), (topic, FrameKind::Snapshot));
        f.data
    }

    fn next_frame(&mut self, within: Duration) -> Option<Frame> {
        let rx = self.client.events();
        let end = Instant::now() + within;
        while Instant::now() < end {
            match rx.try_recv() {
                Ok(CoreEvent::Frame(f)) => return Some(f),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        None
    }

    /// Frames up to and including the first for which `stop` is true, but no longer than `within`.
    pub fn until(&mut self, within: Duration, mut stop: impl FnMut(&Frame) -> bool) -> Vec<Frame> {
        let mut out = Vec::new();
        let end = Instant::now() + within;
        while let Some(f) = self.next_frame(end.saturating_duration_since(Instant::now())) {
            let last = stop(&f);
            out.push(f);
            if last {
                return out;
            }
        }
        panic!("timed out waiting for the expected frame, got: {out:?}");
    }

    /// All frames within the wait window (after a mutation the core gets time to deliver deltas).
    pub fn drain(&mut self, window: Duration) -> Vec<Frame> {
        let mut out = Vec::new();
        let end = Instant::now() + window;
        while let Some(f) = self.next_frame(end.saturating_duration_since(Instant::now())) {
            out.push(f);
        }
        out
    }
}
