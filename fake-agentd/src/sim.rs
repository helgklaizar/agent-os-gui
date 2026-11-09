//! Frame sources: periodic `bar.sys` and the scenario player (on the virtual clock).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::FrameType;
use crate::scenario::{Action, Scenario, StepKind};
use crate::state::Shared;

/// Random walk of metrics (xorshift64*: the seed sets the sequence, `rand` is not needed).
/// Every step is nonzero (1..4), at a boundary the step reflects: the value changes in every frame, disk too.
struct Walk {
    state: u64,
    /// cpu, gpu, ram, disk, temp.
    values: [i64; 5],
}

const RANGES: [(i64, i64); 5] = [(1, 99), (1, 99), (1, 99), (1, 99), (30, 90)];

impl Walk {
    fn new(seed: u64) -> Walk {
        Walk {
            state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            values: [14, 20, 66, 38, 38],
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn step(&mut self) -> Value {
        for (i, (lo, hi)) in RANGES.iter().copied().enumerate() {
            let r = self.next() >> 33;
            let size = (r % 4) as i64 + 1;
            let delta = if (r >> 8).is_multiple_of(2) {
                size
            } else {
                -size
            };
            let next = self.values[i] + delta;
            self.values[i] = if (lo..=hi).contains(&next) {
                next
            } else {
                self.values[i] - delta
            };
        }
        let [cpu, gpu, ram, disk, temp] = self.values;
        json!({ "cpu": cpu, "gpu": gpu, "ram": ram, "disk": disk, "temp": temp })
    }
}

pub async fn ticker(shared: Arc<Shared>, period: Duration, seed: u64) {
    let step = (period.as_millis() as u64).max(1);
    let mut walk = Walk::new(seed);
    let mut n = 1u64;
    loop {
        shared.clock.sleep_until(n * step).await;
        if shared.hub.is_no_data("bar.sys") {
            shared.hub.no_data("bar.sys");
        } else {
            shared.hub.emit("bar.sys", FrameType::Delta, walk.step());
        }
        n += 1;
    }
}

pub async fn play(shared: Arc<Shared>, scenario: Scenario) {
    for step in scenario.steps {
        shared.clock.sleep_until(step.at_ms).await;
        match step.kind {
            StepKind::Emit { topic, kind, data } => shared.hub.emit(&topic, kind, data),
            StepKind::Action(Action::DropWs) => shared.hub.drop_ws(),
            StepKind::Action(Action::Restore) => shared.hub.restore(),
            StepKind::Action(Action::NoData(topic)) => shared.hub.no_data(&topic),
            StepKind::Event(e) => {
                let ev = crate::events_db::NewEvent {
                    kind: &e.kind,
                    sub: &e.sub,
                    level: e.level.as_deref(),
                    text: &e.text,
                    session_id: e.session_id,
                    source: e.source.as_deref(),
                    dedup_key: None,
                    ts: shared.clock.now_unix(),
                };
                if let Err(err) = crate::events_ops::add(&shared, &ev) {
                    tracing::error!(error = %err, "scenario event not written");
                }
            }
        }
    }
}
