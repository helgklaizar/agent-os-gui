//! Agent memory (`docs/reference/db.md` 3.10): writing a fact with the `memory_write` tool through the dispatcher (permissions and log are there too)
//! and persona modes `own | shared | none`. Reading facts into the reply context is `ChatContext::facts`.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{ActionCall, ActionExecutor, Clock, ExecError, PortFuture};

/// Fact text limit, bytes.
pub const MAX_FACT_BYTES: usize = 4096;
/// Writes per minute per session (protection from a flood of facts in one reply).
pub const MAX_FACTS_PER_MINUTE: i64 = 5;
/// How many facts go into the reply context.
pub const CONTEXT_FACTS: i64 = 30;

#[derive(Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// Persona mode `none`.
    Disabled,
    Invalid,
    RateLimited,
    Unavailable,
}

pub trait MemoryStore: Send + Sync + 'static {
    /// Writes a fact to the session by its effective persona mode; `Ok(id)` — the record was created.
    fn add(
        &self,
        session_id: i64,
        text: String,
        now: i64,
    ) -> PortFuture<'_, Result<i64, MemoryError>>;
}

/// Executor of the `memory_write` action: `args = {text}`.
pub struct MemoryExecutor {
    store: Arc<dyn MemoryStore>,
    clock: Arc<dyn Clock>,
}

impl MemoryExecutor {
    #[must_use]
    pub fn new(store: Arc<dyn MemoryStore>, clock: Arc<dyn Clock>) -> MemoryExecutor {
        MemoryExecutor { store, clock }
    }
}

impl ActionExecutor for MemoryExecutor {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        Box::pin(async move {
            let text = call.args["text"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty() && t.len() <= MAX_FACT_BYTES)
                .ok_or(ExecError {
                    code: "invalid_args",
                })?;
            match self
                .store
                .add(call.session_id, text.to_string(), self.clock.now_unix())
                .await
            {
                Ok(id) => Ok(json!({"fact_id": id})),
                Err(MemoryError::Disabled) => Err(ExecError {
                    code: "memory_disabled",
                }),
                Err(MemoryError::Invalid) => Err(ExecError {
                    code: "invalid_args",
                }),
                Err(MemoryError::RateLimited) => Err(ExecError {
                    code: "rate_limited",
                }),
                Err(MemoryError::Unavailable) => Err(ExecError {
                    code: "unavailable",
                }),
            }
        })
    }
}
