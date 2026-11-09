//! Startup before the DB is ready: while the runner applies migrations the server is already listening and answers `/health` with status
//! `migrating` (docs/reference/api.md section 2.1). If there are no migrations, the server comes up after the DB is opened.

use std::sync::{Arc, Mutex};

use agentd_api::{MigrationStatus, MigrationView, Server, ServerConfig};
use agentd_db::RunnerEvent;
use anyhow::{Context, Result};
use tokio::runtime::Handle;

struct State {
    server: Option<Server>,
    from: i64,
    to: i64,
}

pub struct Boot {
    cfg: ServerConfig,
    handle: Handle,
    state: Mutex<State>,
}

fn step(version: i64, name: &str) -> String {
    format!("{version:04}_{name}")
}

impl Boot {
    #[must_use]
    pub fn new(cfg: ServerConfig, handle: Handle) -> Arc<Boot> {
        Arc::new(Boot {
            cfg,
            handle,
            state: Mutex::new(State {
                server: None,
                from: 0,
                to: 0,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runner event handler: called from the DB thread, so the server is brought up via `Handle::block_on`.
    pub fn on_event(&self, e: &RunnerEvent) {
        let mut st = self.lock();
        match e {
            RunnerEvent::Started { from, to } => {
                st.from = *from;
                st.to = *to;
                match self
                    .handle
                    .block_on(Server::start_booting(self.cfg.clone()))
                {
                    Ok(s) => {
                        s.health().set_migration(Some(view(
                            MigrationStatus::Running,
                            *from,
                            *to,
                            None,
                            Some(0.0),
                        )));
                        st.server = Some(s);
                    }
                    // Without a socket migrations still run: the error will surface later, on a normal start.
                    Err(err) => {
                        tracing::error!(scope = "migrator", event = "early_listen_failed", kind = ?err.kind(), "socket for the migration period not opened")
                    }
                }
            }
            RunnerEvent::Applied { version, name } => {
                let span = (st.to - st.from).max(1);
                let progress = f64::from(i32::try_from(version - st.from).unwrap_or(0))
                    / f64::from(i32::try_from(span).unwrap_or(1));
                if let Some(s) = &st.server {
                    s.health().set_migration(Some(view(
                        MigrationStatus::Running,
                        st.from,
                        st.to,
                        Some(step(*version, name)),
                        Some(progress.clamp(0.0, 1.0)),
                    )));
                }
            }
            RunnerEvent::Finished { version } => {
                if let Some(s) = &st.server {
                    s.health().set_migration(Some(view(
                        MigrationStatus::Done,
                        st.from,
                        *version,
                        None,
                        Some(1.0),
                    )));
                }
            }
            RunnerEvent::Error { .. } => {
                if let Some(s) = &st.server {
                    s.health().set_migration(Some(view(
                        MigrationStatus::Failed,
                        st.from,
                        st.to,
                        None,
                        None,
                    )));
                }
            }
        }
    }

    /// The server already brought up for the migration period, or a new one.
    pub async fn server(&self) -> Result<Server> {
        if let Some(s) = self.lock().server.take() {
            return Ok(s);
        }
        Server::start_booting(self.cfg.clone())
            .await
            .with_context(|| format!("socket {}", self.cfg.socket_path.display()))
    }

    /// Whether the server was brought up for the migration period (needed to show `failed` and not drop off the air).
    #[must_use]
    pub fn take_started(&self) -> Option<Server> {
        self.lock().server.take()
    }
}

fn view(
    status: MigrationStatus,
    from: i64,
    to: i64,
    step: Option<String>,
    progress: Option<f64>,
) -> MigrationView {
    MigrationView {
        status,
        from,
        to,
        step,
        progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    fn health(sock: &std::path::Path) -> serde_json::Value {
        let mut s = UnixStream::connect(sock).unwrap();
        s.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let body = out.split_once("\r\n\r\n").unwrap().1;
        // the body may arrive in chunks: take from the first `{` to the last `}`
        let (a, b) = (body.find('{').unwrap(), body.rfind('}').unwrap());
        serde_json::from_str(&body[a..=b]).unwrap()
    }

    /// Runner events bring the server up before the DB is ready, and `/health` shows migration progress.
    #[tokio::test(flavor = "multi_thread")]
    async fn runner_events_drive_health_migrating_then_ok() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("agentd.sock");
        let boot = Boot::new(ServerConfig::new(&sock), Handle::current());
        assert!(!sock.exists(), "no socket before the first event");
        let b = boot.clone();
        tokio::task::spawn_blocking(move || {
            b.on_event(&RunnerEvent::Started { from: 0, to: 4 });
        })
        .await
        .unwrap();
        let h = health(&sock);
        assert_eq!(h["status"], "migrating");
        assert_eq!(h["migration"]["status"], "running");
        assert_eq!(
            (h["migration"]["from"].clone(), h["migration"]["to"].clone()),
            (0.into(), 4.into())
        );
        let b = boot.clone();
        tokio::task::spawn_blocking(move || {
            b.on_event(&RunnerEvent::Applied {
                version: 2,
                name: "chat".into(),
            });
        })
        .await
        .unwrap();
        let h = health(&sock);
        assert_eq!(h["migration"]["step"], "0002_chat");
        assert_eq!(h["migration"]["progress"], 0.5);
        let b = boot.clone();
        tokio::task::spawn_blocking(move || {
            b.on_event(&RunnerEvent::Error {
                code: "migration_failed",
            })
        })
        .await
        .unwrap();
        assert_eq!(health(&sock)["migration"]["status"], "failed");
        let server = boot.server().await.unwrap();
        assert_eq!(
            server.socket_path(),
            sock,
            "the same server, not a second one"
        );
        server.shutdown().await;
    }
}
