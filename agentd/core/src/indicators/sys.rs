//! System indicators from `bar.sys` metrics: `cpu`, `ram`, `disk`, `temp`. Thresholds: percentages
//! 50/80, temperature 60/80; no value — `nodata`.

use serde_json::json;

use super::{IndicatorSource, IndicatorState, Reading};
use crate::{PortFuture, SysSnapshot};

pub struct SysIndicator {
    id: &'static str,
    snap: SysSnapshot,
    warn: f64,
    bad: f64,
    /// Test build only: while the file is absent the indicator answers `nodata` (on Linux a real disk must not produce events
    /// in other tests).
    gate: Option<std::path::PathBuf>,
}

impl SysIndicator {
    /// Indicators `cpu`, `ram`, `disk`, `temp` on a shared measurement.
    #[must_use]
    pub fn all(snap: &SysSnapshot) -> Vec<SysIndicator> {
        [
            ("cpu", 50.0, 80.0),
            ("ram", 50.0, 80.0),
            ("disk", 50.0, 80.0),
            ("temp", 60.0, 80.0),
        ]
        .into_iter()
        .map(|(id, warn, bad)| SysIndicator {
            id,
            snap: snap.clone(),
            warn,
            bad,
            gate: None,
        })
        .collect()
    }

    /// The indicator works only while the `gate` file exists.
    #[must_use]
    pub fn gated(mut self, gate: Option<std::path::PathBuf>) -> SysIndicator {
        self.gate = gate;
        self
    }
}

/// State by value and thresholds: below `warn` — ok, from `warn` — yellow, from `bad` — red.
#[must_use]
pub fn grade(v: Option<f64>, warn: f64, bad: f64) -> IndicatorState {
    match v {
        None => IndicatorState::Nodata,
        Some(x) if x >= bad => IndicatorState::Bad,
        Some(x) if x >= warn => IndicatorState::Warn,
        Some(_) => IndicatorState::Ok,
    }
}

impl IndicatorSource for SysIndicator {
    fn id(&self) -> &str {
        self.id
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            let open = self.gate.as_ref().is_none_or(|g| g.exists());
            let v = self.snap.get()[self.id].as_f64().filter(|_| open);
            let mut r = Reading::new(grade(v, self.warn, self.bad));
            r.value = v.map(|x| json!(x));
            r.text = v.map(|x| format!("{x:.1}"));
            r
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert_eq!(grade(None, 50.0, 80.0), IndicatorState::Nodata);
        assert_eq!(grade(Some(49.9), 50.0, 80.0), IndicatorState::Ok);
        assert_eq!(grade(Some(50.0), 50.0, 80.0), IndicatorState::Warn);
        assert_eq!(grade(Some(80.0), 50.0, 80.0), IndicatorState::Bad);
        assert_eq!(grade(Some(59.0), 60.0, 80.0), IndicatorState::Ok);
    }
}
