#![deny(clippy::unwrap_used)]
//! Core `agentd` client: HTTP and WS over a unix socket and TCP, token, outbound queue.
//!
//! The GUI's only door to the core ([api.md](../../../docs/reference/api.md)). Contract types live in [`api`] and
//! are shared by all client consumers (controllers, `ui`, integration tests); `fake-agentd` deliberately does not
//! import them: the mock describes frames itself, and the tests check both sides.

pub mod api;
pub mod blocking;
pub mod chat;
pub mod client;
pub mod core;
pub mod error;
pub mod events;
pub mod http;
pub mod logging;
pub mod queue;
pub mod settings;
pub mod state;
pub mod token;
pub mod tools;
pub mod transport;
mod tree;
pub mod ws;

mod session;
mod supervisor;

pub use crate::core::{CoreClient, CoreConfig, CoreEvent};
pub use client::HttpClient;
pub use error::ClientError;
pub use state::{ConnState, OfflineReason, ReconnectPolicy};
pub use token::{Token, TokenSource};
pub use transport::Endpoint;
