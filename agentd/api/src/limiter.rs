//! Failed-attempt limit over TCP: 5 per 60 s per address, then `429` with `Retry-After` (ADR-0003).
//! The bucket is the client address; token values are not remembered, the number of buckets is bounded.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

const WINDOW_MS: u64 = 60_000;
const MAX_FAILURES: usize = 5;
/// Cap on the number of addresses in memory; on overflow the address with the oldest failure is evicted.
const MAX_KEYS: usize = 10_000;

#[derive(Debug, Default)]
pub(crate) struct Limiter {
    failures: HashMap<IpAddr, VecDeque<u64>>,
}

impl Limiter {
    /// `Err(seconds)`: the address is paused; at least 1.
    pub(crate) fn check(&mut self, key: IpAddr, now_ms: u64) -> Result<(), u64> {
        let Some(q) = self.failures.get_mut(&key) else {
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

    pub(crate) fn record_failure(&mut self, key: IpAddr, now_ms: u64) {
        if self.failures.len() >= MAX_KEYS / 2 {
            self.prune(now_ms);
        }
        if self.failures.len() >= MAX_KEYS && !self.failures.contains_key(&key) {
            let oldest = self
                .failures
                .iter()
                .filter_map(|(k, q)| q.back().map(|t| (*t, *k)))
                .min()
                .map(|(_, k)| k);
            if let Some(k) = oldest {
                self.failures.remove(&k);
            }
        }
        self.failures.entry(key).or_default().push_back(now_ms);
    }

    fn prune(&mut self, now_ms: u64) {
        self.failures.retain(|_, q| {
            q.back()
                .is_some_and(|t| now_ms.saturating_sub(*t) < WINDOW_MS)
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.failures.len()
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{Limiter, MAX_KEYS};

    fn ip(n: u32) -> IpAddr {
        IpAddr::from(n.to_be_bytes())
    }

    #[test]
    fn five_failures_then_pause_with_retry_after() {
        let mut l = Limiter::default();
        for i in 0..5 {
            assert!(l.check(ip(1), 1000 + i).is_ok());
            l.record_failure(ip(1), 1000 + i);
        }
        assert_eq!(l.check(ip(1), 2000), Err(59));
        assert!(l.check(ip(2), 2000).is_ok(), "another address");
        assert!(l.check(ip(1), 61_000).is_ok(), "window expired");
    }

    #[test]
    fn retry_after_is_at_least_one_second() {
        let mut l = Limiter::default();
        for _ in 0..5 {
            l.record_failure(ip(1), 0);
        }
        assert_eq!(l.check(ip(1), 59_999), Err(1));
    }

    #[test]
    fn memory_is_bounded_and_expired_entries_are_pruned() {
        let mut l = Limiter::default();
        for n in 0..(MAX_KEYS as u32 + 500) {
            l.record_failure(ip(n), u64::from(n));
        }
        assert!(l.len() <= MAX_KEYS, "{}", l.len());
        l.record_failure(ip(u32::MAX), 10_000_000);
        for n in 0..(MAX_KEYS as u32 / 2) {
            l.record_failure(ip(n), 10_000_001);
        }
        assert!(
            l.len() <= MAX_KEYS / 2 + 2,
            "expired addresses purged: {}",
            l.len()
        );
    }
}
