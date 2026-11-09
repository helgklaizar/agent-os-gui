//! Worker thread for theme exchange with the core: GTK is not blocked, writes go strictly in order.
//!
//! The first theme read is retried with a growing pause while the core is unavailable (up to 30 s, like the client's reconnect);
//! while there is no connection the cache from `ui.json` is used. Writes: consecutively queued choices collapse into the last one.

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use async_channel::Receiver;
use controllers::Theme;
use core_client::ReconnectPolicy;
use core_client::blocking::BlockingClient;

use crate::theme_sync::{SyncError, patch_theme, read_theme};

/// What the thread reports to the main loop.
#[derive(Debug)]
pub enum Reply {
    /// Theme value in the core (the core wins over the cache).
    CoreTheme(Theme),
    /// The choice write failed; the choice stays local.
    PatchFailed(SyncError),
}

pub struct CoreWorker {
    jobs: mpsc::Sender<Theme>,
}

impl CoreWorker {
    /// Starts the thread: reads the core theme (with retries) and writes the user's choices.
    pub fn start(core: Arc<BlockingClient>) -> (CoreWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Theme>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("theme-sync".into())
            .spawn(move || {
                let policy = ReconnectPolicy::default();
                let mut fetch_attempt = Some(0u32);
                loop {
                    let wait = match fetch_attempt {
                        Some(0) => Duration::ZERO,
                        Some(n) => policy.delay(n - 1, 0.0),
                        None => Duration::from_secs(3600),
                    };
                    match jobs_rx.recv_timeout(wait) {
                        Ok(mut theme) => {
                            while let Ok(newer) = jobs_rx.try_recv() {
                                theme = newer;
                            }
                            if let Err(e) = patch_theme(core.as_ref(), theme)
                                && reply_tx.send_blocking(Reply::PatchFailed(e)).is_err()
                            {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) if fetch_attempt.is_some() => {
                            match read_theme(core.as_ref()) {
                                Ok(theme) => {
                                    fetch_attempt = None;
                                    if reply_tx.send_blocking(Reply::CoreTheme(theme)).is_err() {
                                        return;
                                    }
                                }
                                Err(_) => fetch_attempt = fetch_attempt.map(|n| n + 1),
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "theme-sync thread not started: {e}");
        }
        (CoreWorker { jobs }, reply_rx)
    }

    /// Writes the choice to the core; a send error means the thread has already stopped (the choice stays local).
    pub fn patch(&self, theme: Theme) {
        if self.jobs.send(theme).is_err() {
            gtk4::glib::g_warning!("agent-shell", "theme-sync thread stopped, theme not sent");
        }
    }
}
