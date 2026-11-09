//! Public entry point: a client with its own tokio runtime on a separate thread and channels outward.

use std::thread::JoinHandle;
use std::time::Duration;

use async_channel::{Receiver, Sender};

use crate::api::{ApiError, Frame, Migration};
use crate::error::ClientError;
use crate::queue::{Outgoing, Target};
use crate::state::{ConnState, OfflineReason, ReconnectPolicy};
use crate::supervisor;
use crate::token::TokenSource;
use crate::transport::Endpoint;

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub endpoint: Endpoint,
    pub token: TokenSource,
    pub reconnect: ReconnectPolicy,
    /// `ping` period (api.md section 7: 15 s); two consecutive missed `pong`s mean a disconnect.
    pub ping_interval: Duration,
    pub request_timeout: Duration,
    /// How long to wait for snapshots after reconnecting before sending the queue.
    pub sync_timeout: Duration,
}

impl CoreConfig {
    pub fn new(endpoint: Endpoint, token: TokenSource) -> CoreConfig {
        CoreConfig {
            endpoint,
            token,
            reconnect: ReconnectPolicy::default(),
            ping_interval: Duration::from_secs(15),
            request_timeout: Duration::from_secs(10),
            sync_timeout: Duration::from_secs(5),
        }
    }
}

/// Events for the consumer (in `ui`, a bridge to the GTK main thread).
#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    State(ConnState),
    /// Reason for the final stop; comes before `State(Offline)`.
    OfflineReason(OfflineReason),
    Frame(Frame),
    /// The core is applying DB migrations: not a connection loss, no "No connection" banner needed (api.md section 2.1).
    Migrating(Option<Migration>),
    /// A queue item was accepted by the core (`duplicate` means a repeat of the same `client_id`).
    Delivered {
        client_id: String,
        duplicate: bool,
    },
    /// The core rejected the item (`4xx`); it is removed from the queue.
    Failed {
        client_id: String,
        error: ApiError,
    },
}

pub(crate) enum Command {
    Subscribe(String),
    Unsubscribe(String),
    Enqueue(Outgoing),
    Shutdown,
}

/// Core client: its own thread with a tokio runtime, only channels outward (`ui` receives events on the GTK main thread).
pub struct CoreClient {
    events: Receiver<CoreEvent>,
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

fn valid_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= 128
        && topic
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

impl CoreClient {
    /// Reads the token (for `TokenSource::Fd` waits for end of data) and starts the thread. Connecting happens inside it;
    /// progress is visible through `CoreEvent::State` events.
    pub fn start(config: CoreConfig) -> Result<CoreClient, ClientError> {
        let token = config.token.read()?;
        let (ev_tx, events) = async_channel::unbounded();
        let (commands, cmd_rx) = async_channel::unbounded();
        let thread = std::thread::Builder::new()
            .name("core-client".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    tracing::error!("core-client runtime not created");
                    return;
                };
                rt.block_on(supervisor::run(config, token, cmd_rx, ev_tx));
            })
            .map_err(|_| ClientError::Protocol("core-client thread not started".into()))?;
        Ok(CoreClient {
            events,
            commands,
            thread: Some(thread),
        })
    }

    pub fn events(&self) -> Receiver<CoreEvent> {
        self.events.clone()
    }

    fn send(&self, cmd: Command) -> Result<(), ClientError> {
        self.commands
            .try_send(cmd)
            .map_err(|_| ClientError::Stopped)
    }

    pub fn subscribe(&self, topic: &str) -> Result<(), ClientError> {
        if !valid_topic(topic) {
            return Err(ClientError::Protocol("invalid topic name".into()));
        }
        self.send(Command::Subscribe(topic.to_string()))
    }

    pub fn unsubscribe(&self, topic: &str) -> Result<(), ClientError> {
        if !valid_topic(topic) {
            return Err(ClientError::Protocol("invalid topic name".into()));
        }
        self.send(Command::Unsubscribe(topic.to_string()))
    }

    /// Queues `echo` with a new `client_id` (UUID v7) and returns it.
    pub fn enqueue_echo(&self, text: &str) -> Result<String, ClientError> {
        let id = uuid::Uuid::now_v7().to_string();
        self.enqueue_echo_with_id(&id, text)?;
        Ok(id)
    }

    pub fn enqueue_echo_with_id(&self, client_id: &str, text: &str) -> Result<(), ClientError> {
        self.send(Command::Enqueue(Outgoing {
            client_id: client_id.to_string(),
            text: text.to_string(),
            target: Target::Echo,
        }))
    }

    /// Queues a chat message: `POST /v1/sessions/{id}/messages` goes out with this `client_id`,
    /// a retry after a connection loss is safe (api.md sections 5.2, 7). Attachments are uploaded before queueing.
    pub fn enqueue_message(
        &self,
        session_id: i64,
        client_id: &str,
        body_md: &str,
        attachment_ids: &[i64],
    ) -> Result<(), ClientError> {
        self.send(Command::Enqueue(Outgoing {
            client_id: client_id.to_string(),
            text: body_md.to_string(),
            target: Target::Message {
                session_id,
                attachment_ids: attachment_ids.to_vec(),
            },
        }))
    }

    /// Stops the thread and waits for it to finish.
    pub fn shutdown(self) {}
}

impl Drop for CoreClient {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("core-client thread terminated abnormally");
        }
    }
}
