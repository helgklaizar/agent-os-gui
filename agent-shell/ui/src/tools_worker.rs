//! Worker thread for core calls from tools: tab commands and personas. GTK is not blocked: jobs run in
//! order, results reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use controllers::CoreCommand;
use core_client::blocking::BlockingClient;
use serde_json::Value;

use crate::tools_exec::{ToolsReply, execute};
use crate::tree_exec;

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Exec(CoreCommand),
    /// `GET /v1/personas` for choosing the session persona.
    Personas,
}

#[derive(Debug)]
pub enum Reply {
    Tools(Vec<ToolsReply>),
    Personas(Vec<Value>),
    /// The command was not executed: a reason without the token, paths or response bodies.
    Failed(String),
}

pub struct ToolsWorker {
    jobs: mpsc::Sender<Job>,
}

impl ToolsWorker {
    pub fn start(core: Arc<BlockingClient>) -> (ToolsWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("tools-worker".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let reply = match job {
                        Job::Exec(cmd) => {
                            // tree commands (folder, persona, thread, delete) are executed by `tree_exec`
                            let done = core.run(|http| async {
                                match execute(http, &cmd).await {
                                    Some(r) => Some(r),
                                    None => tree_exec::execute(http, &cmd)
                                        .await
                                        .map(|r| r.map(|()| Vec::new())),
                                }
                            });
                            match done {
                                Some(Ok(replies)) if replies.is_empty() => None,
                                Some(Ok(replies)) => Some(Reply::Tools(replies)),
                                Some(Err(e)) => Some(Reply::Failed(e)),
                                None => {
                                    Some(Reply::Failed("command is not a tools command".into()))
                                }
                            }
                        }
                        Job::Personas => match core.list_personas() {
                            Ok(list) => Some(Reply::Personas(list)),
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
            gtk4::glib::g_warning!("agent-shell", "tools-worker thread not started: {e}");
        }
        (ToolsWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            gtk4::glib::g_warning!("agent-shell", "tools-worker stopped, job dropped");
        }
    }
}
