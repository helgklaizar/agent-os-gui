//! Worker thread for core calls from the feed: marking as read and reading the subtype registry.
//! GTK is not blocked: jobs run in order, results reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use controllers::CoreCommand;
use core_client::blocking::BlockingClient;
use serde_json::Value;

use crate::notif_exec::execute;

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Exec(CoreCommand),
    /// `GET /v1/events/subtypes`.
    Subtypes,
}

#[derive(Debug)]
pub enum Reply {
    Subtypes(Vec<Value>),
    /// The command was not executed: a reason without the token or response bodies.
    Failed(String),
}

pub struct NotifWorker {
    jobs: mpsc::Sender<Job>,
}

impl NotifWorker {
    pub fn start(core: Arc<BlockingClient>) -> (NotifWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("notif-worker".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let reply = match job {
                        Job::Exec(cmd) => match core.run(|http| execute(http, &cmd)) {
                            Some(Err(e)) => Some(Reply::Failed(e)),
                            _ => None,
                        },
                        Job::Subtypes => match core.event_subtypes() {
                            Ok(list) => Some(Reply::Subtypes(list)),
                            Err(e) => Some(Reply::Failed(e.to_string())),
                        },
                    };
                    if let Some(reply) = reply
                        && reply_tx.send_blocking(reply).is_err()
                    {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "notif-worker thread not started: {e}");
        }
        (NotifWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            gtk4::glib::g_warning!("agent-shell", "notif-worker stopped, job dropped");
        }
    }
}
