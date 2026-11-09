//! Control of the test channel provider (implemented only by `scripted-channel` from the `test-support` build): connection mode, send outcomes, incoming messages.

use serde_json::Value;

use crate::PortFuture;

pub trait ChannelControl: Send + Sync + 'static {
    /// Applies commands from the `POST /_test/channel` body; the result is the provider state (`sends` etc.).
    fn apply(&self, body: Value) -> PortFuture<'_, Value>;
}
