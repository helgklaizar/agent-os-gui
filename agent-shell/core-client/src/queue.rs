//! Outbound queue with `client_id` (api.md section 7): order is preserved, no duplicates.

use std::collections::VecDeque;

/// Where a queue item goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Service command `echo` (`fake-agentd` only).
    Echo,
    /// `POST /v1/sessions/{id}/messages`: text and already uploaded attachments.
    Message {
        session_id: i64,
        attachment_ids: Vec<i64>,
    },
}

/// An outbound queue item with a `client_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub client_id: String,
    pub text: String,
    pub target: Target,
}

#[derive(Debug, Default)]
pub struct OutboundQueue {
    items: VecDeque<Outgoing>,
}

impl OutboundQueue {
    pub fn new() -> OutboundQueue {
        OutboundQueue::default()
    }

    /// Appends to the end; `false` if an item with this `client_id` is already waiting in the queue.
    pub fn push(&mut self, item: Outgoing) -> bool {
        if self.items.iter().any(|i| i.client_id == item.client_id) {
            return false;
        }
        self.items.push_back(item);
        true
    }

    pub fn front(&self) -> Option<&Outgoing> {
        self.items.front()
    }

    pub fn pop_front(&mut self) -> Option<Outgoing> {
        self.items.pop_front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
