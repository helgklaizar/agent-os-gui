//! Worker thread for core calls from chat: sending with attachment upload, decisions, personas, permissions.
//! GTK is not blocked: jobs run in order, results reach the main loop through a channel.

use std::sync::Arc;
use std::sync::mpsc;

use async_channel::Receiver;
use controllers::CoreCommand;
use controllers::chat_types::AttachmentRef;
use core_client::CoreClient;
use core_client::blocking::BlockingClient;
use serde_json::Value;

use crate::chat_exec::{DecisionOutcome, decide_outcome, upload_all};
use crate::tree_exec::execute;

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Exec(CoreCommand),
    Personas,
    Permissions,
    Send {
        session_id: i64,
        client_id: String,
        body_md: String,
        attachments: Vec<AttachmentRef>,
    },
}

#[derive(Debug)]
pub enum Reply {
    Personas(Vec<Value>),
    Permissions(Value),
    Decision {
        request_id: i64,
        outcome: DecisionOutcome,
    },
    /// The command was not executed: a reason without the token, paths or response bodies.
    Failed(String),
    /// The message send was not queued (attachment upload, stopped client).
    SendFailed {
        client_id: String,
        why: String,
    },
}

pub struct ChatWorker {
    jobs: mpsc::Sender<Job>,
}

fn run(core: &BlockingClient, client: &CoreClient, job: Job) -> Option<Reply> {
    match job {
        Job::Personas => core.list_personas().ok().map(Reply::Personas),
        Job::Permissions => core.list_all_permissions().ok().map(Reply::Permissions),
        Job::Send {
            session_id,
            client_id,
            body_md,
            attachments,
        } => {
            let ids = match core.run(|h| upload_all(h, session_id, &attachments)) {
                Ok(ids) => ids,
                Err(why) => return Some(Reply::SendFailed { client_id, why }),
            };
            client
                .enqueue_message(session_id, &client_id, &body_md, &ids)
                .err()
                .map(|e| Reply::SendFailed {
                    client_id,
                    why: e.to_string(),
                })
        }
        Job::Exec(CoreCommand::DecidePermission {
            request_id,
            decision,
            rule,
        }) => {
            let outcome = core.run(|h| decide_outcome(h, request_id, decision, rule));
            Some(Reply::Decision {
                request_id,
                outcome,
            })
        }
        Job::Exec(cmd) => match core.run(|h| execute(h, &cmd)) {
            Some(Err(e)) => Some(Reply::Failed(e)),
            _ => None,
        },
    }
}

impl ChatWorker {
    pub fn start(
        core: Arc<BlockingClient>,
        client: Arc<CoreClient>,
    ) -> (ChatWorker, Receiver<Reply>) {
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = async_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("chat-worker".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    if let Some(reply) = run(&core, &client, job)
                        && reply_tx.send_blocking(reply).is_err()
                    {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            gtk4::glib::g_warning!("agent-shell", "chat-worker thread not started: {e}");
        }
        (ChatWorker { jobs }, reply_rx)
    }

    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            gtk4::glib::g_warning!("agent-shell", "chat-worker stopped, job dropped");
        }
    }
}
