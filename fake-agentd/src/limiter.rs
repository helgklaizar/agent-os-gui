//! Failed-attempt limit: 5 per 60 s per source, then a pause with `Retry-After`.

use std::collections::{HashMap, VecDeque};

const WINDOW_MS: u64 = 60_000;
const MAX_FAILURES: usize = 5;

#[derive(Debug, Default)]
pub struct Limiter {
    failures: HashMap<String, VecDeque<u64>>,
}

impl Limiter {
    /// `Err(seconds)` - the source is paused; at least 1 s.
    pub fn check(&mut self, key: &str, now_ms: u64) -> Result<(), u64> {
        let Some(q) = self.failures.get_mut(key) else {
            return Ok(());
        };
        while q
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= WINDOW_MS)
        {
            q.pop_front();
        }
        if q.len() < MAX_FAILURES {
            return Ok(());
        }
        let until = q.front().map_or(0, |t| t + WINDOW_MS);
        Err(until.saturating_sub(now_ms).div_ceil(1000).max(1))
    }

    pub fn record_failure(&mut self, key: &str, now_ms: u64) {
        self.failures
            .entry(key.to_string())
            .or_default()
            .push_back(now_ms);
    }
}
