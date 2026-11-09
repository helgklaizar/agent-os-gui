//! Executor of the `send` action: the message goes out only after the gate decision (`PolicyGate` stands before
//! the executor), through the provider relay; the result is written to `gw_messages` (`direction = out`), the log gets `reversible = 0`.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Channels, Outcome};
use crate::{
    ActionCall, ActionExecutor, Clock, Direction, ExecError, Gateway, Ingest, PortFuture,
    channel_of,
};

pub struct ChannelSend {
    channels: Channels,
    gateway: Gateway,
    clock: Arc<dyn Clock>,
}

impl ChannelSend {
    #[must_use]
    pub fn new(channels: Channels, gateway: Gateway, clock: Arc<dyn Clock>) -> ChannelSend {
        ChannelSend {
            channels,
            gateway,
            clock,
        }
    }
}

fn text_of(args: &Value) -> Option<&str> {
    ["text", "body", "message"]
        .iter()
        .find_map(|k| args.get(k).and_then(Value::as_str))
        .filter(|t| !t.trim().is_empty())
}

impl ActionExecutor for ChannelSend {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        Box::pin(async move {
            let (channel, to) = call
                .target
                .split_once(':')
                .filter(|(c, h)| !c.is_empty() && !h.is_empty())
                .ok_or(ExecError {
                    code: "invalid_args",
                })?;
            let (acc, _) = self
                .channels
                .accounts()
                .await
                .into_iter()
                .find(|(a, st)| st == "connected" && channel_of(&a.kind) == channel)
                .ok_or(ExecError {
                    code: "no_connector",
                })?;
            let text = text_of(&call.args).ok_or(ExecError {
                code: "invalid_args",
            })?;
            let key = format!("out-{}", uuid::Uuid::now_v7());
            let outcome = self
                .channels
                .registry()
                .send(&acc, to, text, &key)
                .await
                .ok_or(ExecError {
                    code: "no_connector",
                })?;
            match outcome {
                Outcome::Delivered { ext_id } => {
                    let row = Ingest {
                        account_id: acc.id,
                        ext_id: ext_id.unwrap_or(key),
                        direction: Direction::Out,
                        handle: to.to_string(),
                        body: text.to_string(),
                        ts: self.clock.now_unix(),
                    };
                    if self.gateway.ingest(row).await.is_err() {
                        tracing::warn!(
                            scope = "gateway",
                            event = "out_not_archived",
                            "sent message not recorded in the archive"
                        );
                    }
                    Ok(json!({"delivered": true}))
                }
                Outcome::Rejected(_) => Err(ExecError { code: "rejected" }),
                Outcome::Unknown => Err(ExecError {
                    code: "delivery_unknown",
                }),
            }
        })
    }
}
