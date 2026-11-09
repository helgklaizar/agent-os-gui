//! Connection state and reconnect policy (api.md section 7).

use std::time::Duration;

/// The single set of connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    /// Final stop without retries (see [`OfflineReason`]); the attempt counter never leads here.
    Offline,
    /// The core answers `core_down` or `status != ok` ("Core does not start"); retries continue.
    CoreUnavailable,
}

/// Why the client stopped reconnecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineReason {
    /// `401` or WS close with code `4401`.
    TokenRejected,
    /// `403` or WS close with code `4403`: scope, `Origin`, `Host`.
    Forbidden,
    /// The core's `api.major` is not 1, or close `4426`.
    ApiVersionUnsupported,
}

/// Pause series: 1, 2, 4, 8, 16, 30 s, ceiling 30 s, jitter only downward (the ceiling is never exceeded), no attempt limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectPolicy {
    pub base: Duration,
    pub max: Duration,
    /// Share of pause reduction, 0..1 (working value 0.2).
    pub jitter: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        ReconnectPolicy {
            base: Duration::from_secs(1),
            max: Duration::from_secs(30),
            jitter: 0.2,
        }
    }
}

impl ReconnectPolicy {
    /// Pause before attempt number `attempt` (from zero); `r` from `[0, 1)` sets the jitter.
    pub fn delay(&self, attempt: u32, r: f64) -> Duration {
        let grown = self.base.as_secs_f64() * 2f64.powi(attempt.min(30) as i32);
        let capped = grown.min(self.max.as_secs_f64());
        let lowered = capped * (1.0 - self.jitter.clamp(0.0, 1.0) * r.clamp(0.0, 1.0));
        Duration::from_secs_f64(lowered)
    }
}
