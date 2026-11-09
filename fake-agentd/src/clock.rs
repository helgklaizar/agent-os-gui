//! Virtual clock: real or manual. Everything that depends on time (token expiry, the attempt-limit window,
//! scenario steps, the `bar.sys` period) follows it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockMode {
    #[default]
    Real,
    /// Time stands still until a test calls `advance` (`--clock manual`).
    Manual,
}

pub struct Clock {
    mode: ClockMode,
    start: Instant,
    base_unix: i64,
    offset_ms: AtomicU64,
    changed: watch::Sender<u64>,
}

impl Clock {
    pub fn new(mode: ClockMode) -> Clock {
        let base_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (changed, _) = watch::channel(0);
        Clock {
            mode,
            start: Instant::now(),
            base_unix,
            offset_ms: AtomicU64::new(0),
            changed,
        }
    }

    /// Milliseconds of virtual time since start.
    pub fn now_ms(&self) -> u64 {
        let real = match self.mode {
            ClockMode::Real => self.start.elapsed().as_millis() as u64,
            ClockMode::Manual => 0,
        };
        real + self.offset_ms.load(Ordering::SeqCst)
    }

    /// Unix seconds of virtual time.
    pub fn now_unix(&self) -> i64 {
        self.base_unix + (self.now_ms() / 1000) as i64
    }

    /// Shifts forward; wakes those waiting in [`sleep_until`](Self::sleep_until).
    pub fn advance(&self, by: Duration) {
        let total = self
            .offset_ms
            .fetch_add(by.as_millis() as u64, Ordering::SeqCst)
            + by.as_millis() as u64;
        self.changed.send_replace(total);
    }

    /// Waits until virtual time reaches `target_ms`.
    pub async fn sleep_until(&self, target_ms: u64) {
        let mut rx = self.changed.subscribe();
        loop {
            let now = self.now_ms();
            if now >= target_ms {
                return;
            }
            match self.mode {
                ClockMode::Manual => {
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
                ClockMode::Real => {
                    let wait = Duration::from_millis((target_ms - now).min(50));
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = rx.changed() => {}
                    }
                }
            }
        }
    }
}
