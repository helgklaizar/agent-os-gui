//! The "incoming message accepted" port: triggers subscribe to it. The gateway calls it only for
//! incoming messages not isolated by the observer.

use crate::PortFuture;

/// An incoming channel message as triggers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMsg {
    /// Channel (`tg`, `wa`, `mail`), also the trigger `source`.
    pub channel: String,
    /// Sender in the channel (`handle`) and the name of the matched contact.
    pub handle: String,
    pub contact: Option<String>,
    pub body: String,
    /// `gw_messages` row: the run idempotency key.
    pub gw_id: i64,
}

pub trait IncomingHook: Send + Sync + 'static {
    fn on_incoming<'a>(&'a self, msg: &'a IncomingMsg) -> PortFuture<'a, ()>;
}
