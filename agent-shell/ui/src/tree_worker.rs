//! Worker thread for core calls from the tree: commands from dialogs and row actions, reading personas.
//! GTK is not blocked: jobs run in order, results reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use controllers::CoreCommand;
use core_client::blocking::BlockingClient;
use serde_json::Value;

use crate::tree_exec::execute;

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Exec(CoreCommand),
    /// Read personas (`GET /v1/personas`).
    Personas,
}

#[derive(Debug)]
pub enum Reply {
    Personas(Vec<Value>),
    /// The command was not executed: a reason without the token or response bodies.
    Failed(String),
}

pub struct TreeWorker {
    jobs: mpsc::Sender<Job>,
}

impl TreeWorker {
    pub fn start(core: Arc<BlockingClient>) -> (TreeWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("tree-worker".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let reply = match job {
                        Job::Exec(cmd) => match core.run(|http| execute(http, &cmd)) {
                            Some(Err(e)) => Some(Reply::Failed(e)),
                            _ => None,
                        },
                        Job::Personas => core.list_personas().ok().map(Reply::Personas),
                    };
                    if let Some(reply) = reply
                        && reply_tx.send_blocking(reply).is_err()
                    {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "tree-worker thread not started: {e}");
        }
        (TreeWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            gtk4::glib::g_warning!("agent-shell", "tree-worker stopped, job dropped");
        }
    }
}
