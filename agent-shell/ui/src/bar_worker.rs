//! Worker thread for core calls from the bar: version for About, `balance.fallback`, Logout.
//! GTK is not blocked: jobs run in order, replies reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use core_client::blocking::BlockingClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Read `balance.fallback`.
    Fallback,
    /// Read the core version.
    About,
    /// `POST /v1/logout`.
    Logout,
}

#[derive(Debug)]
pub enum Reply {
    /// `balance.fallback` value; `None` means not read.
    Fallback(Option<String>),
    /// Core version; `None` means no data.
    About(Option<String>),
    /// Logout result: `Err` is a reason without the token or response bodies.
    Logout(Result<(), String>),
}

pub struct BarWorker {
    jobs: mpsc::Sender<Job>,
}

impl BarWorker {
    pub fn start(core: Arc<BlockingClient>) -> (BarWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("bar-worker".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let reply = match job {
                        Job::Fallback => Reply::Fallback(
                            core.personas()
                                .ok()
                                .and_then(|p| p.balance)
                                .and_then(|b| b.fallback),
                        ),
                        Job::About => Reply::About(core.health().ok().map(|h| h.version)),
                        Job::Logout => Reply::Logout(core.logout().map_err(|e| e.to_string())),
                    };
                    if reply_tx.send_blocking(reply).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "bar-worker thread not started: {e}");
        }
        (BarWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            gtk4::glib::g_warning!("agent-shell", "bar-worker stopped, job {job:?} dropped");
        }
    }
}
