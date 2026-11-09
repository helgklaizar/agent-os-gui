//! Core time source (the `Clock` port). The controllable `ManualClock` exists only in the test build.

use std::fmt::Debug;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Debug + Send + Sync + 'static {
    /// Seconds since the epoch (`*_at` fields in the DB).
    fn now_unix(&self) -> i64;

    /// Milliseconds since the epoch (limit windows).
    fn now_ms(&self) -> u64;

    /// Moves the controllable clock forward; a real clock cannot be moved (`false`).
    fn advance_secs(&self, _secs: i64) -> bool {
        false
    }
}

/// The controllable clock if the core is built with `test-support`; in a release build — `None` (the clock is always real).
#[must_use]
pub fn manual_clock(start_unix: i64) -> Option<std::sync::Arc<dyn Clock>> {
    #[cfg(feature = "test-support")]
    {
        Some(std::sync::Arc::new(ManualClock::new(start_unix)))
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = start_unix;
        None
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        i64::try_from(self.now_ms() / 1000).unwrap_or(i64::MAX)
    }

    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// Controllable clock of the test build: starts at a given moment and moves only by `advance_secs`.
#[cfg(feature = "test-support")]
#[derive(Debug)]
pub struct ManualClock(std::sync::atomic::AtomicI64);

#[cfg(feature = "test-support")]
impl ManualClock {
    #[must_use]
    pub fn new(start_unix: i64) -> ManualClock {
        ManualClock(std::sync::atomic::AtomicI64::new(
            start_unix.saturating_mul(1000),
        ))
    }
}

#[cfg(feature = "test-support")]
impl Clock for ManualClock {
    fn now_unix(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst) / 1000
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0)
    }

    fn advance_secs(&self, secs: i64) -> bool {
        self.0.fetch_add(
            secs.saturating_mul(1000),
            std::sync::atomic::Ordering::SeqCst,
        );
        true
    }
}
