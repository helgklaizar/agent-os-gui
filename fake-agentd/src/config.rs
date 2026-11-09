//! Mock launch settings.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::clock::ClockMode;
use crate::scenario::Scenario;

/// Core state for client checks (api.md sections 2.1, 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreMode {
    #[default]
    Ok,
    /// Everything except `/v1/health` answers `503 core_down`; health reports `status = starting`.
    Down,
    /// Everything except `/v1/health` answers `503 migrating`; health reports `status = migrating`.
    Migrating,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) socket: Option<PathBuf>,
    pub(crate) tcp: Option<SocketAddr>,
    pub(crate) clock: ClockMode,
    pub(crate) scenario: Option<Scenario>,
    pub(crate) bar_sys_period: Option<Duration>,
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) token_file: Option<PathBuf>,
    pub(crate) api_major: u32,
    pub(crate) db: Option<PathBuf>,
    pub(crate) bar_seed: u64,
    pub(crate) reply_delay: Duration,
    pub(crate) event_fixtures: bool,
}

impl Config {
    /// Temporary directory with the socket inside `FakeAgentd`; no TCP, real clock, `bar.sys` every 2 s.
    pub fn temp() -> Config {
        Config {
            socket: None,
            tcp: None,
            clock: ClockMode::Real,
            scenario: None,
            bar_sys_period: Some(Duration::from_secs(2)),
            allowed_origins: Vec::new(),
            token_file: None,
            api_major: 1,
            db: None,
            bar_seed: 1,
            reply_delay: Duration::ZERO,
            event_fixtures: true,
        }
    }

    /// Additionally listens on `127.0.0.1` on a random port.
    pub fn with_tcp(mut self) -> Config {
        self.tcp = Some(SocketAddr::from(([127, 0, 0, 1], 0)));
        self
    }

    /// TCP on the given address (for the binary: `--tcp`).
    pub fn tcp_addr(mut self, addr: SocketAddr) -> Config {
        self.tcp = Some(addr);
        self
    }

    /// Custom socket path instead of `agentd.sock` in the temporary directory (for the binary: `--socket`).
    pub fn socket(mut self, path: PathBuf) -> Config {
        self.socket = Some(path);
        self
    }

    pub fn manual_clock(mut self) -> Config {
        self.clock = ClockMode::Manual;
        self
    }

    pub fn scenario(mut self, scenario: Scenario) -> Config {
        self.scenario = Some(scenario);
        self
    }

    pub fn bar_sys_period(mut self, period: Duration) -> Config {
        self.bar_sys_period = Some(period);
        self
    }

    /// No periodic `bar.sys` output (data comes from a scenario or `emit`).
    pub fn no_ticker(mut self) -> Config {
        self.bar_sys_period = None;
        self
    }

    /// Allowed `Origin` for TCP (tunnel address).
    pub fn allow_origin(mut self, origin: &str) -> Config {
        self.allowed_origins.push(origin.to_string());
        self
    }

    /// Where to write the `gui` token (`AGENTD_TOKEN_FILE`, tests and manual work only).
    pub fn token_file(mut self, path: PathBuf) -> Config {
        self.token_file = Some(path);
        self
    }

    /// DB file (for the binary: `--db`); without it `sessions.db` in the mock's temporary directory.
    pub fn db(mut self, path: PathBuf) -> Config {
        self.db = Some(path);
        self
    }

    /// Seed of the `bar.sys` random walk: one seed gives the same frame sequence.
    pub fn bar_seed(mut self, seed: u64) -> Config {
        self.bar_seed = seed;
        self
    }

    /// Pause before the agent reply ("simulation mode"): the binary sets 2.5 s, tests default to no pause.
    pub fn reply_delay(mut self, delay: Duration) -> Config {
        self.reply_delay = delay;
        self
    }

    /// No event fixtures: the feed is empty until a test or scenario fills it.
    pub fn no_event_fixtures(mut self) -> Config {
        self.event_fixtures = false;
        self
    }

    /// Which `api.major` `/v1/health` returns.
    pub fn api_major(mut self, major: u32) -> Config {
        self.api_major = major;
        self
    }
}
