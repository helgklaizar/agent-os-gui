//! Bar colour levels and thresholds. Values are rounded to integers (half-up) before comparison.

/// Indicator state for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Level {
    /// Plain text: metric below the yellow threshold.
    #[default]
    Neutral,
    Ok,
    Warn,
    Bad,
    /// No data: grey.
    NoData,
}

/// Round to integers, half up (`40.5 → 41`).
pub fn round_pct(value: f64) -> i64 {
    // 1e-9 absorbs binary arithmetic noise: (1 − 0.595) · 100 = 40.49999999999999, not 40.5
    (value + 0.5 + 1e-9).floor() as i64
}

/// Remaining quota in percent: `> 40` green, `> 15` yellow, otherwise red.
pub fn quota_remaining_level(remaining_pct: f64) -> Level {
    match round_pct(remaining_pct) {
        41.. => Level::Ok,
        16..=40 => Level::Warn,
        _ => Level::Bad,
    }
}

/// Burn rate: `≤ 1×` green, `< 1.5×` yellow, otherwise red.
pub fn quota_rate_level(rate: f64) -> Level {
    if rate <= 1.0 {
        Level::Ok
    } else if rate < 1.5 {
        Level::Warn
    } else {
        Level::Bad
    }
}

/// CPU, GPU, RAM, disk: `< 50` neutral, `< 80` yellow, otherwise red.
pub fn load_level(pct: f64) -> Level {
    match round_pct(pct) {
        ..=49 => Level::Neutral,
        50..=79 => Level::Warn,
        _ => Level::Bad,
    }
}

/// Temperature, °C: `< 60` neutral, `< 80` yellow, otherwise red.
pub fn temp_level(celsius: f64) -> Level {
    match round_pct(celsius) {
        ..=59 => Level::Neutral,
        60..=79 => Level::Warn,
        _ => Level::Bad,
    }
}

/// Local model GPU/VRAM load: `< 80` green, `< 95` yellow, otherwise red.
pub fn local_load_level(pct: f64) -> Level {
    match round_pct(pct) {
        ..=79 => Level::Ok,
        80..=94 => Level::Warn,
        _ => Level::Bad,
    }
}

impl Level {
    /// Worst of two colours: red > yellow > green > neutral; "no data" yields to any other.
    pub fn worst(self, other: Level) -> Level {
        fn rank(l: Level) -> u8 {
            match l {
                Level::NoData => 0,
                Level::Neutral => 1,
                Level::Ok => 2,
                Level::Warn => 3,
                Level::Bad => 4,
            }
        }
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}
