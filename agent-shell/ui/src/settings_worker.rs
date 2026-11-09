//! Worker thread for core calls from the Settings dialog: GTK is not blocked, commands run strictly in order,
//! replies (one or several) reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use controllers::settings_types::{Cmd, Reply};
use core_client::blocking::BlockingClient;

use crate::settings_exec::execute;

pub struct SettingsWorker {
    jobs: mpsc::Sender<Cmd>,
}

impl SettingsWorker {
    pub fn start(core: Arc<BlockingClient>) -> (SettingsWorker, Receiver<Vec<Reply>>) {
        let (jobs, jobs_rx) = mpsc::channel::<Cmd>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("settings-worker".into())
            .spawn(move || {
                while let Ok(cmd) = jobs_rx.recv() {
                    let replies = core.run(|http| execute(http, &cmd));
                    if reply_tx.send_blocking(replies).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "settings-worker thread not started: {e}");
        }
        (SettingsWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, cmd: Cmd) {
        if self.jobs.send(cmd).is_err() {
            gtk4::glib::g_warning!("agent-shell", "settings-worker stopped, command dropped");
        }
    }
}
