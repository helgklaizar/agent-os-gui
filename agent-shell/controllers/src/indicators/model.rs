//! Indicator manifest and colour rules: `{id, source, interval, thresholds, click_action}`.

/// Indicator state (in the UI: "yellow", "red", "no data", "unknown").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Ok,
    Warn,
    Bad,
    NoData,
    Unknown,
}

impl State {
    pub fn parse(s: &str) -> Option<State> {
        [
            State::Ok,
            State::Warn,
            State::Bad,
            State::NoData,
            State::Unknown,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Warn => "warn",
            State::Bad => "bad",
            State::NoData => "nodata",
            State::Unknown => "unknown",
        }
    }
}

/// Indicator group in Settings and in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    System,
    Ai,
    Messages,
    Infra,
    /// Module indicators (external servers and services): outside the 0.1 criteria, enabled together with the module.
    Module,
}

/// Rule mapping a value to a state. `inclusive` means the boundary is in the range (`≥` and `≤`), otherwise strict (`>` and `<`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    /// No thresholds: the indicator only shows the value.
    None,
    /// Higher is worse.
    Above {
        warn: Option<f64>,
        bad: Option<f64>,
        inclusive: bool,
    },
    /// Lower is worse.
    Below {
        warn: Option<f64>,
        bad: Option<f64>,
        inclusive: bool,
    },
    /// Flag: `0` is normal, any positive value is the `on` state.
    Flag { on: State },
}

/// Click action (tooltip with details, as for quotas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    Tooltip,
}

/// Indicator manifest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manifest {
    pub id: &'static str,
    pub group: Group,
    /// Data source (a daemon or the OS bus); the GUI does not poll it.
    pub source: &'static str,
    /// Poll period, ms; `None` means the source is event-driven.
    pub interval_ms: Option<u64>,
    pub rule: Rule,
    pub click_action: ClickAction,
}

fn crossed(value: f64, limit: f64, above: bool, inclusive: bool) -> bool {
    match (above, inclusive) {
        (true, true) => value >= limit,
        (true, false) => value > limit,
        (false, true) => value <= limit,
        (false, false) => value < limit,
    }
}

/// State from the value and thresholds; no value (or `NaN`) means "no data".
pub fn state_for(m: &Manifest, value: Option<f64>) -> State {
    let Some(v) = value.filter(|v| !v.is_nan()) else {
        return State::NoData;
    };
    let (warn, bad, above, inclusive) = match m.rule {
        Rule::None => return State::Ok,
        Rule::Flag { on } => return if v > 0.0 { on } else { State::Ok },
        Rule::Above {
            warn,
            bad,
            inclusive,
        } => (warn, bad, true, inclusive),
        Rule::Below {
            warn,
            bad,
            inclusive,
        } => (warn, bad, false, inclusive),
    };
    if bad.is_some_and(|t| crossed(v, t, above, inclusive)) {
        State::Bad
    } else if warn.is_some_and(|t| crossed(v, t, above, inclusive)) {
        State::Warn
    } else {
        State::Ok
    }
}

/// State of the `infra.agentd` indicator from the GUI-to-core link state.
/// Assumption: no duration threshold is specified for `reconnecting` and `offline`, so the state is derived from the link kind:
/// reconnecting is yellow, offline and `core_unavailable` are red.
pub fn agentd_state(conn: crate::types::ConnState) -> State {
    use crate::types::ConnState::*;
    match conn {
        Connected => State::Ok,
        Connecting => State::Unknown,
        Reconnecting => State::Warn,
        Offline | CoreUnavailable => State::Bad,
    }
}
