//! Shared API state and the core services that get plugged in.

use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};

use agentd_core::{
    Authenticator, ChatStore, Clock, QuotaStore, SecretStore, SettingsStore, StatusSource,
    TokenAdmin, TreeStore,
};

use crate::error::ApiError;
use crate::health::HealthControl;
use crate::hub::Hub;
use crate::limiter::Limiter;
use crate::metrics::Metrics;
use crate::server::TcpControl;

/// Allowed `Host` and `Origin` over TCP (tunnel addresses), besides its own `127.0.0.1:port` and `localhost:port`.
#[derive(Debug, Clone, Default)]
pub(crate) struct OriginPolicy {
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub trusted_proxies: Vec<IpAddr>,
    /// Browser client directory (`--web-root`); `None` means statics are not served (ADR-0010).
    pub web_root: Option<std::path::PathBuf>,
    /// Origins with dev-CORS (`--dev-cors`); they are also allowed by the `Origin` check.
    pub dev_cors: Vec<String>,
}

/// Services plugged in by `bin` once the DB is ready. A missing service is `503 core_down` on its routes.
#[derive(Clone)]
pub struct Services {
    pub auth: Arc<dyn Authenticator>,
    pub tokens: Option<Arc<dyn TokenAdmin>>,
    pub settings: Option<Arc<dyn SettingsStore>>,
    pub status: Option<Arc<dyn StatusSource>>,
    pub tree: Option<Arc<dyn TreeStore>>,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub chat: Option<Arc<dyn ChatStore>>,
    pub quotas: Option<Arc<dyn QuotaStore>>,
    /// Event feed (A5-03).
    pub events: Option<Arc<dyn agentd_core::EventStore>>,
    /// Channel account connections, inbound polling (A5-06).
    pub channels: Option<agentd_core::Channels>,
    /// Test channel control: set only in the `test-support` build, `None` in release.
    pub channel_control: Option<Arc<dyn agentd_core::ChannelControl>>,
    /// Gateway: inbound intake and routing (A5-05).
    pub gateway: Option<agentd_core::Gateway>,
    /// Bar indicator engine (A5-04).
    pub indicators: Option<agentd_core::IndicatorEngine>,
    /// Action log: token issue and revocation, decisions, permission changes.
    pub audit: Option<agentd_core::Audit>,
    /// Permission engine: three-level rules, writes with the "tighten only" check.
    pub engine: Option<agentd_core::PermEngine>,
    /// Browser commands and `tool-results` intake.
    pub browser: Option<agentd_core::BrowserBridge>,
    /// Permission gate: permissions, observer, permission requests.
    pub gate: Option<agentd_core::PolicyGate>,
    /// Observer: checks actions and texts; its unavailability puts `/health` in `degraded`.
    pub observer: Option<Arc<dyn agentd_core::Observer>>,
    /// Model replies: queue, stream, stop.
    pub responder: Option<agentd_core::ChatService>,
    /// Tasks and schedule (A5-09).
    pub tasks: Option<agentd_core::Tasks>,
    /// Triggers on inbound messages (A5-10).
    pub triggers: Option<agentd_core::Triggers>,
    /// Unified search (A6-08).
    pub search: Option<Arc<dyn agentd_core::SearchStore>>,
    /// `AGENT.md` and `memory/` files (A6-10).
    pub files: Option<Arc<dyn agentd_core::FileStore>>,
    /// Tunnels (A6-05).
    pub tunnels: Option<Arc<dyn agentd_core::TunnelStore>>,
    /// Tunnel process statuses (A6-06).
    pub tunnel_statuses: Option<agentd_core::TunnelStatuses>,
    /// Modules (A6-03).
    pub modules: Option<Arc<dyn agentd_core::ModuleStore>>,
    /// Encrypted session export (A6-11).
    pub exporter: Option<Arc<dyn agentd_core::SessionExporter>>,
    /// MCP memory source `memory_mcp` (A6-09, optional).
    pub memory_mcp: Option<agentd_core::McpMemorySource>,
    /// Latest hardware metrics sample (A7-04).
    pub sys: Option<agentd_core::SysSnapshot>,
    /// Ending the user session (A7-10).
    pub logout: Option<Arc<dyn agentd_core::SessionCloser>>,
    /// Latest weather frame (A7-06).
    pub weather: Option<agentd_core::WeatherSnapshot>,
    /// Downloading a module manifest by URL (A6-03).
    pub module_fetcher: Option<Arc<dyn agentd_core::ModuleFetcher>>,
}

impl Services {
    #[must_use]
    pub fn new(auth: Arc<dyn Authenticator>) -> Services {
        Services {
            auth,
            tokens: None,
            settings: None,
            status: None,
            tree: None,
            secrets: None,
            chat: None,
            quotas: None,
            events: None,
            indicators: None,
            gateway: None,
            channels: None,
            channel_control: None,
            audit: None,
            engine: None,
            observer: None,
            gate: None,
            browser: None,
            responder: None,
            tasks: None,
            triggers: None,
            search: None,
            files: None,
            tunnels: None,
            tunnel_statuses: None,
            modules: None,
            exporter: None,
            memory_mcp: None,
            sys: None,
            logout: None,
            weather: None,
            module_fetcher: None,
        }
    }
}

pub(crate) struct ApiState {
    pub policy: OriginPolicy,
    pub services: RwLock<Option<Arc<Services>>>,
    pub clock: Arc<dyn Clock>,
    pub limiter: Mutex<Limiter>,
    pub health: HealthControl,
    pub metrics: Metrics,
    pub tcp: TcpControl,
    pub hub: Hub,
    pub ping_interval: std::time::Duration,
}

impl ApiState {
    pub(crate) fn services(&self) -> Option<Arc<Services>> {
        self.services
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Response while services are not plugged in: migration is running (or failed) -> `503 migrating`, otherwise `503 core_down`.
    pub(crate) fn not_ready(&self) -> ApiError {
        let migration = self.health.migration_json();
        let migrating = migration
            .get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s != "done");
        if migrating {
            ApiError::migrating(migration)
        } else {
            ApiError::core_down("Core is starting")
        }
    }
}
