//! "Runs out in" forecast: shown only if the ETA is before the reset.

use crate::i18n::tr_args;

/// Forecast result for the quota tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forecast {
    /// Runs out in this many seconds (before the reset).
    Eta(i64),
    /// Usage fits the window: "Enough until reset".
    EnoughUntilReset,
    /// Nothing to say: the limit or the reset is unknown.
    Unknown,
}

/// `ETA = (1 − used) / (rate / window_h)` hours; compared with the reset.
pub fn forecast(
    now_unix: i64,
    used: Option<f64>,
    rate: Option<f64>,
    window_h: Option<f64>,
    reset_at: Option<i64>,
) -> Forecast {
    let (Some(used), Some(rate), Some(window), Some(reset)) = (used, rate, window_h, reset_at)
    else {
        return Forecast::Unknown;
    };
    if window <= 0.0 || reset <= now_unix {
        return Forecast::Unknown;
    }
    if rate <= 1.0 {
        return Forecast::EnoughUntilReset;
    }
    let eta = ((1.0 - used).max(0.0) / (rate / window) * 3600.0).round() as i64;
    if now_unix + eta < reset {
        Forecast::Eta(eta)
    } else {
        Forecast::EnoughUntilReset
    }
}

/// Duration for captions: "1 h 21 min", "45 min", "2 h", "less than a minute".
pub fn duration_text(secs: i64) -> String {
    let minutes = secs.max(0) / 60;
    let (h, m) = ((minutes / 60).to_string(), (minutes % 60).to_string());
    match (minutes / 60, minutes % 60) {
        (0, 0) => tr_args("duration-lt-minute", &[]),
        (0, _) => tr_args("duration-m", &[("m", &m)]),
        (_, 0) => tr_args("duration-h", &[("h", &h)]),
        _ => tr_args("duration-hm", &[("h", &h), ("m", &m)]),
    }
}
