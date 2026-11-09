//! Operational background tasks of `serve` (A7-10): `sd_notify` (`READY`, `WATCHDOG`, `STOPPING`) and the Healthchecks heartbeat.

use super::watchdog::Notifier;

pub(super) struct Ops {
    notifier: Option<Notifier>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Ops {
    /// Started after the core is ready: `READY=1`, periodic `WATCHDOG=1`, heartbeat (if an address is set).
    pub(super) fn start() -> Self {
        let notifier = Notifier::from_env();
        let mut tasks: Vec<_> = notifier.clone().map(Notifier::start).into_iter().collect();
        if let Some(cfg) = crate::heartbeat::from_env(&|k| std::env::var(k).ok()) {
            tasks.push(crate::heartbeat::spawn(
                cfg,
                agentd_providers::HttpClient::new(agentd_providers::Timeouts::default()),
            ));
        }
        Ops { notifier, tasks }
    }

    pub(super) fn stop(self) {
        if let Some(n) = &self.notifier {
            n.stopping();
        }
        for t in self.tasks {
            t.abort();
        }
    }
}
