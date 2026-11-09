//! Reply text accumulator: flush to the DB and the `message.delta` delta with `offset`.

use serde_json::json;

use super::{Inner, topic};

/// Accumulated but unwritten text and the length of what is already written.
pub(super) struct Sink<'a> {
    pub inner: &'a Inner,
    pub session_id: i64,
    pub message_id: i64,
    pub buf: String,
    pub len: usize,
    pub alive: bool,
}

impl Sink<'_> {
    pub async fn flush(&mut self) {
        if self.buf.is_empty() || !self.alive {
            return;
        }
        let text = std::mem::take(&mut self.buf);
        match self.inner.store.append(self.message_id, text.clone()).await {
            Ok(new_len) => {
                let offset = self.len;
                if new_len != offset + text.len() {
                    tracing::warn!(
                        scope = "chat",
                        event = "offset_mismatch",
                        "DB length did not match the accumulated length"
                    );
                }
                self.len = new_len;
                self.inner.notifier.publish_event(
                    &topic(self.session_id),
                    json!({"kind": "message.delta", "message_id": self.message_id, "offset": offset, "text": text}),
                );
            }
            Err(_) => self.alive = false,
        }
    }
}
