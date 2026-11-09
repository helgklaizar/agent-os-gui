//! `ChatStore` for `ChatRepo`.

use agentd_core::{
    AccountRow, Attachment, ChatContext, ChatError, ChatResult, ChatStore, HistoryItem, PortFuture,
    Posted, UserMessage,
};
use serde_json::{Value, json};

use super::{ChatRepo, context, messages, unavailable};

const SNAPSHOT_PAGE: i64 = 50;

fn found<T>(v: Option<T>) -> ChatResult<T> {
    v.ok_or(ChatError::NotFound)
}

impl ChatStore for ChatRepo {
    fn post_user(&self, session_id: i64, msg: UserMessage) -> PortFuture<'_, ChatResult<Posted>> {
        Box::pin(async move {
            let atts = self.attachments.resolve(session_id, &msg.attachment_ids)?;
            let now = self.now();
            self.db
                .write(move |tx| {
                    messages::post_user(tx, now, session_id, &msg.client_id, &msg.body_md, &atts)
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn list(
        &self,
        session_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> PortFuture<'_, ChatResult<Value>> {
        Box::pin(async move {
            let rows = self
                .db
                .read(move |c| {
                    Ok(if messages::session_exists(c, session_id)? {
                        Some(messages::page(c, session_id, before, limit)?)
                    } else {
                        None
                    })
                })
                .await
                .map_err(|e| unavailable(&e))?;
            Ok(Value::Array(found(rows)?))
        })
    }

    fn session_snapshot(&self, session_id: i64) -> PortFuture<'_, ChatResult<Value>> {
        Box::pin(async move {
            let snap = self
                .db
                .read(move |c| {
                    let Some(session) = crate::tree::session_json(c, session_id)? else {
                        return Ok(None);
                    };
                    let mut msgs = messages::page(c, session_id, None, SNAPSHOT_PAGE)?;
                    msgs.reverse();
                    let open = crate::perms::open_requests(c, session_id)?;
                    let tasks = crate::tasks::session_tasks(c, session_id)?;
                    Ok(Some(json!({"session": session, "messages": msgs, "open_requests": open, "tasks": tasks})))
                })
                .await
                .map_err(|e| unavailable(&e))?;
            found(snap)
        })
    }

    fn create_agent(&self, session_id: i64, author: String) -> PortFuture<'_, ChatResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.db
                .write(move |tx| messages::create_agent(tx, now, session_id, &author))
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn append(&self, message_id: i64, text: String) -> PortFuture<'_, ChatResult<usize>> {
        Box::pin(async move {
            let n = self
                .db
                .write(move |tx| messages::append(tx, message_id, &text))
                .await
                .map_err(|e| unavailable(&e))?;
            found(n)
        })
    }

    fn finish(
        &self,
        message_id: i64,
        status: &'static str,
        meta: Option<Value>,
    ) -> PortFuture<'_, ChatResult<usize>> {
        Box::pin(async move {
            let n = self
                .db
                .write(move |tx| messages::finish(tx, message_id, status, meta.as_ref()))
                .await
                .map_err(|e| unavailable(&e))?;
            found(n)
        })
    }

    fn create_system(
        &self,
        session_id: i64,
        body_md: String,
        meta: Value,
    ) -> PortFuture<'_, ChatResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.db
                .write(move |tx| messages::create_system(tx, now, session_id, &body_md, &meta))
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn create_tool(
        &self,
        session_id: i64,
        body_md: String,
        meta: Value,
    ) -> PortFuture<'_, ChatResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.db
                .write(move |tx| messages::create_tool(tx, now, session_id, &body_md, &meta))
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn update_card(
        &self,
        request_id: i64,
        patch: Value,
    ) -> PortFuture<'_, ChatResult<Option<Value>>> {
        Box::pin(async move {
            self.db
                .write(move |tx| messages::update_card(tx, request_id, &patch))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn clear(&self, session_id: i64) -> PortFuture<'_, ChatResult<()>> {
        Box::pin(async move {
            self.db
                .write(move |tx| messages::clear(tx, session_id))
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn interrupt_streaming(&self) -> PortFuture<'_, ChatResult<u64>> {
        Box::pin(async move {
            self.db
                .write(messages::interrupt_streaming)
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn context(&self, session_id: i64) -> PortFuture<'_, ChatResult<ChatContext>> {
        Box::pin(async move {
            let root = self.root.clone();
            let ctx = self
                .db
                .read(move |c| context::context(c, &root, session_id))
                .await
                .map_err(|e| unavailable(&e))?;
            found(ctx)
        })
    }

    fn account(&self, id: i64) -> PortFuture<'_, ChatResult<Option<AccountRow>>> {
        Box::pin(async move {
            self.db
                .read(move |c| context::account(c, id))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn history(
        &self,
        session_id: i64,
        before_id: i64,
        with_parent: bool,
        max_bytes: usize,
    ) -> PortFuture<'_, ChatResult<Vec<HistoryItem>>> {
        Box::pin(async move {
            self.db
                .read(move |c| context::history(c, session_id, before_id, with_parent, max_bytes))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn put_attachment(
        &self,
        session_id: i64,
        name: String,
        mime: String,
        data: Vec<u8>,
    ) -> PortFuture<'_, ChatResult<Attachment>> {
        Box::pin(async move {
            let exists = self
                .db
                .read(move |c| messages::session_exists(c, session_id))
                .await
                .map_err(|e| unavailable(&e))?;
            if !exists {
                return Err(ChatError::NotFound);
            }
            self.attachments.put(session_id, &name, &mime, &data)
        })
    }

    fn get_attachment(&self, id: i64) -> PortFuture<'_, ChatResult<(Attachment, Vec<u8>)>> {
        Box::pin(async move { self.attachments.get(id) })
    }
}
