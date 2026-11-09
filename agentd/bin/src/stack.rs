//! Assembly of indicators, gateway and channels for `serve` (A5): shared dependencies and background polling.

use std::sync::Arc;
use std::time::Duration;

use agentd_api::Hub;
use agentd_core::{
    AgentdStatus, AiUsage, Channels, ChatStore, Clock, Events, Gateway, IndicatorEngine, Observer,
    ObserverHealth, QuotaStore, SettingsStore, StatusSource, TreeStore, Unread,
};
use agentd_db::{Db, EventRepo, GatewayRepo, IndicatorRepo};
use agentd_providers::{HttpClient, KeysFile, Timeouts};

use crate::perms::Perms;

pub(crate) struct Stack {
    pub indicators: IndicatorEngine,
    pub gateway: Gateway,
    pub channels: Channels,
    pub control: Option<Arc<dyn agentd_core::ChannelControl>>,
}

/// Shared assembly dependencies.
pub(crate) struct Deps<'a> {
    pub db: &'a Arc<Db>,
    pub clock: &'a Arc<dyn Clock>,
    pub hub: Hub,
    pub settings: Arc<dyn SettingsStore>,
    pub chat: Arc<dyn ChatStore>,
    pub tree: Arc<dyn TreeStore>,
    pub quotas: Arc<dyn QuotaStore>,
    pub events_repo: Arc<EventRepo>,
    pub events: Events,
    pub keys: Arc<KeysFile>,
    pub perms: &'a Perms,
    pub sys: agentd_core::SysSnapshot,
    /// `~/.agentos`: the backup directory is `export/`.
    pub agentos: std::path::PathBuf,
}

pub(crate) fn build(d: Deps<'_>) -> Stack {
    let hub: Arc<Hub> = Arc::new(d.hub.clone());
    let indicators = IndicatorEngine::new(
        Arc::new(IndicatorRepo::new(
            d.db.clone(),
            d.clock.clone(),
            hub.clone(),
        )),
        d.settings.clone(),
    );
    let status: Arc<dyn StatusSource> = d.db.clone();
    indicators.register(Arc::new(AgentdStatus(status)));
    // In a test build system indicators are enabled by the file from `AGENTD_SYS_GATE_FILE` (otherwise the real disk of dev-vm produces events).
    let gate = agentd_core::manual_clock(0)
        .and(std::env::var_os("AGENTD_SYS_GATE_FILE"))
        .map(std::path::PathBuf::from);
    for s in agentd_core::SysIndicator::all(&d.sys) {
        indicators.register(Arc::new(s.gated(gate.clone())));
    }
    let probe: Arc<dyn agentd_core::SystemProbe> =
        Arc::new(crate::sys_probe::FsProbe::new(&d.agentos));
    for s in agentd_core::SystemIndicator::all(&probe, d.clock, &d.settings) {
        indicators.register(Arc::new(s));
    }
    indicators.register(Arc::new(Unread(d.events_repo.clone())));
    indicators.register(Arc::new(AiUsage(d.quotas.clone())));
    let observer: Arc<dyn Observer> = d.perms.observer.clone();
    indicators.register(Arc::new(ObserverHealth(observer.clone())));
    indicators.spawn(Duration::from_secs(5));
    let gateway = Gateway::new(
        Arc::new(GatewayRepo::new(d.db.clone())),
        d.settings.clone(),
        observer,
        d.perms.audit.clone(),
        d.chat.clone(),
        d.tree.clone(),
        d.events.clone(),
        hub.clone(),
    );
    let mut registry = agentd_providers::channels::standard_channels(
        d.keys.clone(),
        HttpClient::new(Timeouts::default()),
    );
    let mut control = None;
    if let Some((provider, ctl)) = agentd_providers::channels::scripted_channel(d.keys.clone()) {
        registry = registry.with(provider);
        control = Some(ctl);
    }
    let channels = Channels::new(
        registry,
        d.tree.clone(),
        gateway.clone(),
        Some(indicators.clone()),
        hub,
        d.clock.clone(),
    );
    channels.spawn(Duration::from_secs(3));
    Stack {
        indicators,
        gateway,
        channels,
        control,
    }
}
