//! Outer client loop: `health` check, connecting, states, reconnect pause (api.md section 7).

use std::collections::BTreeSet;
use std::time::Duration;

use async_channel::{Receiver, Sender};

use crate::api::{ErrorCode, HealthStatus, Migration};
use crate::client::HttpClient;
use crate::core::{Command, CoreConfig, CoreEvent};
use crate::error::ClientError;
use crate::queue::OutboundQueue;
use crate::session::{self, End};
use crate::state::{ConnState, OfflineReason};
use crate::token::Token;
use crate::ws::WsConn;

/// Shared loop state; inside a connection it is owned by [`session`].
pub(crate) struct Core {
    pub cfg: CoreConfig,
    pub token: Token,
    pub http: HttpClient,
    pub subs: BTreeSet<String>,
    pub queue: OutboundQueue,
    pub events: Sender<CoreEvent>,
    pub cmds: Receiver<Command>,
    state: Option<ConnState>,
}

enum Fail {
    Fatal(OfflineReason),
    CoreDown,
    Migrating(Option<Migration>),
    /// Disconnect; the pause is at least `Retry-After` if the core supplied one.
    Lost(Option<Duration>),
}

/// What to do when a command arrives during a pause or in `Offline`.
pub(crate) enum Flow {
    Go,
    Stop,
}

impl Core {
    pub fn emit(&self, ev: CoreEvent) {
        let _ = self.events.try_send(ev);
    }

    pub fn set_state(&mut self, s: ConnState) {
        if self.state != Some(s) {
            self.state = Some(s);
            tracing::info!(scope = "connection", event = "connection.state", state = ?s, "connection state");
            self.emit(CoreEvent::State(s));
        }
    }

    pub fn stopped(&self) -> bool {
        self.events.is_closed()
    }

    pub fn apply(&mut self, cmd: Command) -> Flow {
        match cmd {
            Command::Subscribe(t) => {
                self.subs.insert(t);
            }
            Command::Unsubscribe(t) => {
                self.subs.remove(&t);
            }
            Command::Enqueue(item) => {
                self.queue.push(item);
            }
            Command::Shutdown => return Flow::Stop,
        }
        Flow::Go
    }

    async fn connect(&self) -> Result<WsConn, Fail> {
        let health = self.http.health().await.map_err(|e| classify(&e))?;
        if health.api.major != 1 {
            return Err(Fail::Fatal(OfflineReason::ApiVersionUnsupported));
        }
        match health.status {
            HealthStatus::Starting => return Err(Fail::CoreDown),
            HealthStatus::Migrating => return Err(Fail::Migrating(health.migration)),
            HealthStatus::Ok | HealthStatus::Degraded | HealthStatus::Unknown => {}
        }
        let open = WsConn::connect(&self.cfg.endpoint, Some(&self.token), "/v1/ws");
        match tokio::time::timeout(self.cfg.request_timeout, open).await {
            Ok(Ok(ws)) => Ok(ws),
            Ok(Err(e)) => Err(classify(&e)),
            Err(_) => Err(Fail::Lost(None)),
        }
    }

    /// Waits `delay` while accepting commands. `Stop` means the client was closed or `Shutdown` arrived.
    async fn pause(&mut self, delay: Duration) -> Flow {
        let deadline = tokio::time::Instant::now() + delay;
        loop {
            if self.stopped() {
                return Flow::Stop;
            }
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => return Flow::Go,
                cmd = self.cmds.recv() => match cmd {
                    Ok(c) => {
                        if matches!(self.apply(c), Flow::Stop) {
                            return Flow::Stop;
                        }
                    }
                    Err(_) => return Flow::Stop,
                },
            }
        }
    }

    /// After the final stop commands are accepted but the connection is not restored.
    async fn idle(&mut self) {
        while let Ok(cmd) = self.cmds.recv().await {
            if matches!(self.apply(cmd), Flow::Stop) || self.stopped() {
                return;
            }
        }
    }

    fn fatal(&mut self, reason: OfflineReason) {
        self.emit(CoreEvent::OfflineReason(reason));
        self.set_state(ConnState::Offline);
    }
}

fn classify(e: &ClientError) -> Fail {
    let ClientError::Api(api) = e else {
        return Fail::Lost(None);
    };
    match (api.status, &api.code) {
        (401, _) => Fail::Fatal(OfflineReason::TokenRejected),
        (403, _) => Fail::Fatal(OfflineReason::Forbidden),
        (_, ErrorCode::CoreDown) => Fail::CoreDown,
        (_, ErrorCode::Migrating) => Fail::Migrating(
            api.details
                .get("migration")
                .and_then(|m| serde_json::from_value(m.clone()).ok()),
        ),
        _ => Fail::Lost(api.retry_after),
    }
}

/// Random pause jitter in `[0, 1)`: the low bits of a UUID v7 are random.
fn jitter_r() -> f64 {
    (uuid::Uuid::now_v7().as_u128() & 0x00FF_FFFF) as f64 / 16_777_216.0
}

pub(crate) async fn run(
    cfg: CoreConfig,
    token: Token,
    cmds: Receiver<Command>,
    events: Sender<CoreEvent>,
) {
    let http = HttpClient::new(cfg.endpoint.clone(), Some(token.clone()))
        .with_timeout(cfg.request_timeout);
    let mut core = Core {
        cfg,
        token,
        http,
        subs: BTreeSet::new(),
        queue: OutboundQueue::new(),
        events,
        cmds,
        state: None,
    };
    core.set_state(ConnState::Connecting);
    let mut attempt = 0u32;
    loop {
        if core.stopped() {
            return;
        }
        let mut floor = None;
        match core.connect().await {
            Ok(ws) => {
                attempt = 0;
                core.set_state(ConnState::Connected);
                match session::run(&mut core, ws).await {
                    End::Shutdown => return,
                    End::Lost => core.set_state(ConnState::Reconnecting),
                    End::Fatal(reason) => {
                        core.fatal(reason);
                        core.idle().await;
                        return;
                    }
                }
            }
            Err(Fail::Fatal(reason)) => {
                core.fatal(reason);
                core.idle().await;
                return;
            }
            Err(Fail::CoreDown) => core.set_state(ConnState::CoreUnavailable),
            Err(Fail::Migrating(m)) => core.emit(CoreEvent::Migrating(m)),
            Err(Fail::Lost(retry)) => {
                floor = retry;
                core.set_state(ConnState::Reconnecting);
            }
        }
        let delay = core
            .cfg
            .reconnect
            .delay(attempt, jitter_r())
            .max(floor.unwrap_or_default());
        attempt = attempt.saturating_add(1);
        if matches!(core.pause(delay).await, Flow::Stop) {
            return;
        }
    }
}
