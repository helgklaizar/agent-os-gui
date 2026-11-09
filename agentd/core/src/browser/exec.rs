//! `browse`: action executor through the command bridge.

use serde_json::Value;

use super::{BrowserBridge, BrowserOutcome};
use crate::{ActionCall, ActionExecutor, ExecError, PortFuture};

/// `browse`: a `navigate` command to the session subscribers and waiting for the result from the GUI.
pub struct BrowseExecutor {
    bridge: BrowserBridge,
}

impl BrowseExecutor {
    #[must_use]
    pub fn new(bridge: BrowserBridge) -> BrowseExecutor {
        BrowseExecutor { bridge }
    }
}

impl ActionExecutor for BrowseExecutor {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        Box::pin(async move {
            let (_, mut rx) = self
                .bridge
                .open(call.session_id, "navigate", &call.target)
                .map_err(|code| ExecError { code })?;
            let outcome = loop {
                if let Some(o) = rx.borrow_and_update().clone() {
                    break o;
                }
                if rx.changed().await.is_err() {
                    return Err(ExecError { code: "closed" });
                }
            };
            match outcome {
                BrowserOutcome::Closed("timeout") => Err(ExecError { code: "timeout" }),
                BrowserOutcome::Closed(_) => Err(ExecError { code: "closed" }),
                BrowserOutcome::Result(r) if r.status == "ok" => Ok(r.to_json()),
                BrowserOutcome::Result(r) => Err(ExecError {
                    code: match r.status.as_str() {
                        "timeout" => "timeout",
                        "cancelled" => "cancelled",
                        "denied" => "denied",
                        _ => "browser_error",
                    },
                }),
            }
        })
    }
}
