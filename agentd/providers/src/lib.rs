//! Model providers and key secrets: `keys.env`, Anthropic and OpenAI-compatible clients,
//! and the `scripted` test provider. The core (`agentd-core`) knows only the traits.

pub mod anthropic;
pub mod channels;
mod common;
pub mod http;
mod keys;
mod keys_format;
pub mod limits;
pub mod mcp;
pub mod modules;
pub mod openai;
mod registry;
#[cfg(feature = "test-support")]
pub mod scripted;
pub mod sse;
pub mod weather;

pub use http::{HttpClient, Timeouts};
pub use keys::KeysFile;
pub use mcp::RmcpConnector;
pub use modules::HttpModuleFetcher;
pub use registry::standard_registry;
