//! Quotas by fact (`docs/reference/api.md` 4.4, `docs/reference/db.md` 3.5): store port for usage accounting and provider limits.

use serde_json::Value;

use crate::{LimitWindow, PortFuture, RateLimit, Usage};

/// Model price per million tokens (`models` page).
#[derive(Debug, Clone, PartialEq)]
pub struct Price {
    pub input_per_m: f64,
    pub output_per_m: f64,
    pub currency: String,
}

impl Price {
    #[must_use]
    pub fn cost(&self, u: Usage) -> f64 {
        (u.tokens_in as f64 * self.input_per_m + u.tokens_out as f64 * self.output_per_m)
            / 1_000_000.0
    }
}

/// Outcome of one model reply that must be accounted.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub account_id: i64,
    pub local: bool,
    pub session_id: i64,
    pub rate: Option<RateLimit>,
    pub now: i64,
}

pub trait QuotaStore: Send + Sync + 'static {
    /// Recalculates the account quota after a reply; the result is a `Quota` object for the `bar.quota` delta
    /// (`None` if the account no longer exists).
    fn record(&self, r: UsageRecord) -> PortFuture<'_, Option<Value>>;
    /// `Quota` of all accounts; `session_id` — usage per session instead of per day.
    fn quotas(&self, session_id: Option<i64>) -> PortFuture<'_, Option<Value>>;
}

/// Provider limit window in seconds: the `QuotaLimits` schema gives limits "per minute".
pub const WINDOW_SECS: i64 = 60;

/// The tightest limit window: the one with the smallest remaining share.
#[must_use]
pub fn tightest(r: &RateLimit) -> Option<LimitWindow> {
    let frac = |w: &LimitWindow| w.remaining as f64 / w.limit.max(1) as f64;
    [r.requests, r.tokens]
        .into_iter()
        .flatten()
        .filter(|w| w.limit > 0)
        .min_by(|a, b| frac(a).total_cmp(&frac(b)))
}

/// Share of the limit consumed, 0..1.
#[must_use]
pub fn used_fraction(w: &LimitWindow) -> f64 {
    (1.0 - w.remaining as f64 / w.limit.max(1) as f64).clamp(0.0, 1.0)
}

/// Burn rate, × of the allowed: the whole limit may be spent per window; elapsed time is computed from the reset time.
#[must_use]
pub fn rate_of(w: &LimitWindow, now: i64) -> f64 {
    let to_reset = w
        .reset_at
        .map_or(WINDOW_SECS, |r| (r - now).clamp(0, WINDOW_SECS));
    let elapsed = (WINDOW_SECS - to_reset).max(1);
    used_fraction(w) * WINDOW_SECS as f64 / elapsed as f64
}

/// When the limit runs out at the current rate (unix seconds); only if the reset and usage are known and the end comes
/// before the reset, otherwise `None` ("enough until reset").
#[must_use]
pub fn eta_at(used: f64, rate: f64, at: i64, reset_at: Option<i64>) -> Option<i64> {
    let reset = reset_at?;
    if rate <= 0.0 || used <= 0.0 {
        return None;
    }
    let burn_per_sec = rate / WINDOW_SECS as f64;
    let eta = at + ((1.0 - used) / burn_per_sec).ceil() as i64;
    (eta < reset).then_some(eta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(limit: u64, remaining: u64, reset_in: Option<i64>, now: i64) -> LimitWindow {
        LimitWindow {
            limit,
            remaining,
            reset_at: reset_in.map(|s| now + s),
        }
    }

    #[test]
    fn tightest_picks_smallest_remaining_share() {
        let r = RateLimit {
            requests: Some(w(100, 90, None, 0)),
            tokens: Some(w(1000, 100, None, 0)),
        };
        assert_eq!(tightest(&r).unwrap().limit, 1000);
        assert!(tightest(&RateLimit::default()).is_none());
        let zero = RateLimit {
            requests: Some(w(0, 0, None, 0)),
            tokens: None,
        };
        assert!(tightest(&zero).is_none(), "a zero limit is not a window");
    }

    #[test]
    fn used_and_rate() {
        let win = w(100, 80, Some(30), 1000);
        assert!((used_fraction(&win) - 0.2).abs() < 1e-12);
        // 30 s of 60 elapsed: spent 0.2 → rate 0.4x the allowed.
        assert!((rate_of(&win, 1000) - 0.4).abs() < 1e-12);
        assert_eq!(rate_of(&w(100, 100, Some(30), 0), 0), 0.0);
        // Reset is far (not a one-minute window) or unknown: assume at least a second has passed.
        assert!(rate_of(&w(100, 50, Some(500), 0), 0).is_finite());
        assert!(rate_of(&w(100, 50, None, 0), 0).is_finite());
    }

    #[test]
    fn quota_eta_only_before_reset() {
        let now = 1000;
        let early = w(100, 10, Some(50), now); // 0.9 in 10 s: ends in ~2 s
        let u = used_fraction(&early);
        let r = rate_of(&early, now);
        let eta = eta_at(u, r, now, early.reset_at).unwrap();
        assert!(eta > now && eta < early.reset_at.unwrap(), "{eta}");
        let late = w(100, 90, Some(10), now); // 0.1 in 50 s: the remainder would last 450 s
        let (u, r) = (used_fraction(&late), rate_of(&late, now));
        assert_eq!(eta_at(u, r, now, late.reset_at), None);
        assert_eq!(eta_at(0.0, 0.0, now, Some(now + 5)), None, "nothing spent");
        assert_eq!(eta_at(0.5, 1.0, now, None), None, "reset unknown");
    }

    #[test]
    fn price_cost() {
        let p = Price {
            input_per_m: 3.0,
            output_per_m: 15.0,
            currency: "USD".into(),
        };
        let c = p.cost(Usage {
            tokens_in: 1_000_000,
            tokens_out: 2_000_000,
        });
        assert!((c - 33.0).abs() < 1e-9);
    }
}
