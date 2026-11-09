//! Chat history (`docs/reference/api.md` 5.2, `docs/reference/db.md` 3.2): input types with shape validation and the message and attachment store port.

use serde_json::Value;

use crate::{FieldError, PortFuture};

/// Message body limit, bytes.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Attachment limit, bytes (`docs/reference/api.md` 9.3, working value).
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
/// Limit on the number of attachments per message.
pub const MAX_ATTACHMENTS: usize = 10;
/// History page limit.
pub const MAX_PAGE: i64 = 200;

#[derive(Debug, PartialEq)]
pub enum ChatError {
    NotFound,
    /// Fields failed validation (`422`).
    Invalid(Vec<FieldError>),
    /// `409` with a code (`client_id_conflict`).
    Conflict(&'static str),
    /// `413`.
    TooLarge,
    /// `429`: the session queue is full.
    QueueFull,
    Unavailable,
}

pub type ChatResult<T> = Result<T, ChatError>;

/// Attachment (`Attachment` schema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub id: i64,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

impl Attachment {
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({"id": self.id, "name": self.name, "mime": self.mime, "size": self.size})
    }
}

/// Validated user message: `role = user` only.
#[derive(Debug, Clone, PartialEq)]
pub struct UserMessage {
    pub client_id: String,
    pub body_md: String,
    pub attachment_ids: Vec<i64>,
}

/// Result of `POST /messages`: `Created` — `201`, `Existing` — a `client_id` repeat, `200`.
#[derive(Debug, Clone, PartialEq)]
pub enum Posted {
    Created(Value),
    Existing(Value),
}

/// What is known about the session for the model reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatContext {
    pub session_name: String,
    /// The session's `cfg.model`.
    pub cfg_model: Option<i64>,
    /// Effective persona: name, style, language, signature, model account.
    pub persona_id: Option<i64>,
    pub persona_name: Option<String>,
    pub style: String,
    pub lang: String,
    pub signature: String,
    pub persona_model_acc: Option<i64>,
    /// Context shared with the parent (`cfg.ctx = parent`, the default for a thread).
    pub ctx_parent: bool,
    /// `~/.agentos/AGENT.md`, if present.
    pub agent_md: Option<String>,
    /// Memory facts by persona mode, newest first.
    pub facts: Vec<String>,
}

/// Model account from the DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub secret_ref: Option<String>,
}

/// History message for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryItem {
    pub agent: bool,
    pub text: String,
}

/// Parsing the `POST /sessions/{id}/messages` body: exactly the fields `body_md`, `client_id`, `attachment_ids`; everything else
/// (`role`, `meta`, `status`…) the client cannot set.
pub fn parse_user_message(body: &Value) -> Result<UserMessage, ChatError> {
    let Some(obj) = body.as_object() else {
        return Err(ChatError::Invalid(vec![FieldError::new(
            "",
            "an object is expected",
        )]));
    };
    let mut errs: Vec<FieldError> = obj
        .keys()
        .filter(|k| !matches!(k.as_str(), "body_md" | "client_id" | "attachment_ids"))
        .map(|k| FieldError::new(k, "the field is set by the core or is unknown"))
        .collect();
    let client_id = obj.get("client_id").and_then(Value::as_str);
    if !client_id.is_some_and(valid_client_id) {
        errs.push(FieldError::new(
            "client_id",
            "1 to 128 printable characters",
        ));
    }
    let body_md = obj.get("body_md").and_then(Value::as_str);
    match body_md {
        None => errs.push(FieldError::new("body_md", "a string is required")),
        Some(s) if s.trim().is_empty() && !has_attachments(obj) => {
            errs.push(FieldError::new("body_md", "empty message"));
        }
        Some(_) => {}
    }
    let mut ids = Vec::new();
    match obj.get("attachment_ids") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.len() <= MAX_ATTACHMENTS => {
            for v in a {
                match v.as_i64().filter(|n| *n > 0) {
                    Some(n) if !ids.contains(&n) => ids.push(n),
                    _ => errs.push(FieldError::new(
                        "attachment_ids",
                        "positive integers without repeats",
                    )),
                }
            }
        }
        Some(_) => errs.push(FieldError::new(
            "attachment_ids",
            "an array of up to 10 integers",
        )),
    }
    if !errs.is_empty() {
        return Err(ChatError::Invalid(errs));
    }
    let body_md = body_md.unwrap_or_default();
    if body_md.len() > MAX_BODY_BYTES {
        return Err(ChatError::TooLarge);
    }
    Ok(UserMessage {
        client_id: client_id.unwrap_or_default().to_string(),
        body_md: body_md.to_string(),
        attachment_ids: ids,
    })
}

fn has_attachments(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("attachment_ids")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
}

/// `client_id`: a client UUID or any string of 1–128 printable characters without surrounding spaces (as in `fake-agentd`).
#[must_use]
pub fn valid_client_id(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.chars().all(|c| !c.is_control()) && s.trim() == s
}

/// History store (implemented by `agentd-db`).
pub trait ChatStore: Send + Sync + 'static {
    /// User message; a `client_id` repeat with the same text — `Existing`, with a different one — `Conflict`.
    fn post_user(&self, session_id: i64, msg: UserMessage) -> PortFuture<'_, ChatResult<Posted>>;
    /// History page from newest to oldest.
    fn list(
        &self,
        session_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> PortFuture<'_, ChatResult<Value>>;
    /// `snapshot` of the `session.<id>` topic.
    fn session_snapshot(&self, session_id: i64) -> PortFuture<'_, ChatResult<Value>>;
    /// Agent message in `streaming` status (empty body).
    fn create_agent(&self, session_id: i64, author: String) -> PortFuture<'_, ChatResult<Value>>;
    /// Appends text; the result is the `body_md` length in bytes after the write.
    fn append(&self, message_id: i64, text: String) -> PortFuture<'_, ChatResult<usize>>;
    /// Closes the message; the result is the final `body_md` length in bytes.
    fn finish(
        &self,
        message_id: i64,
        status: &'static str,
        meta: Option<Value>,
    ) -> PortFuture<'_, ChatResult<usize>>;
    /// `role = system` message (permission request card): body and `meta`; the result is the whole message.
    fn create_system(
        &self,
        session_id: i64,
        body_md: String,
        meta: Value,
    ) -> PortFuture<'_, ChatResult<Value>>;
    /// `role = tool` message: the agent action result for the model (body `[tool_result {…}]`) and `meta`.
    fn create_tool(
        &self,
        session_id: i64,
        body_md: String,
        meta: Value,
    ) -> PortFuture<'_, ChatResult<Value>>;
    /// Updates the `meta` of the request card `request_id`: `patch` fields replace same-named ones (`request`, `status`).
    /// The result is the updated message; `None` — no card (a request without a session, or the session was deleted).
    fn update_card(
        &self,
        request_id: i64,
        patch: Value,
    ) -> PortFuture<'_, ChatResult<Option<Value>>>;
    fn clear(&self, session_id: i64) -> PortFuture<'_, ChatResult<()>>;
    /// Core start: `streaming` → `interrupted`; how many rows.
    fn interrupt_streaming(&self) -> PortFuture<'_, ChatResult<u64>>;
    fn context(&self, session_id: i64) -> PortFuture<'_, ChatResult<ChatContext>>;
    fn account(&self, id: i64) -> PortFuture<'_, ChatResult<Option<AccountRow>>>;
    /// History tail for the model (old → new), at most `max_bytes` of text; `with_parent` — the parent's tail first.
    fn history(
        &self,
        session_id: i64,
        before_id: i64,
        with_parent: bool,
        max_bytes: usize,
    ) -> PortFuture<'_, ChatResult<Vec<HistoryItem>>>;

    fn put_attachment(
        &self,
        session_id: i64,
        name: String,
        mime: String,
        data: Vec<u8>,
    ) -> PortFuture<'_, ChatResult<Attachment>>;
    /// Metadata and content.
    fn get_attachment(&self, id: i64) -> PortFuture<'_, ChatResult<(Attachment, Vec<u8>)>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_message_forms() {
        let ok = parse_user_message(&json!({"body_md":"hello","client_id":"c-1"})).unwrap();
        assert_eq!(
            (ok.client_id.as_str(), ok.body_md.as_str()),
            ("c-1", "hello")
        );
        let fields = |v: Value| match parse_user_message(&v) {
            Err(ChatError::Invalid(f)) => f.into_iter().map(|e| e.field).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            fields(json!({"body_md":"x","client_id":"c","role":"agent"})),
            ["role"]
        );
        assert_eq!(
            fields(json!({"body_md":"x","client_id":"c","meta":{}})),
            ["meta"]
        );
        assert_eq!(fields(json!({"body_md":"x"})), ["client_id"]);
        assert_eq!(fields(json!({"body_md":"x","client_id":""})), ["client_id"]);
        assert_eq!(
            fields(json!({"body_md":"x","client_id":"a\nb"})),
            ["client_id"]
        );
        assert_eq!(fields(json!({"client_id":"c"})), ["body_md"]);
        assert_eq!(fields(json!({"body_md":"  ","client_id":"c"})), ["body_md"]);
        assert_eq!(
            fields(json!({"body_md":"x","client_id":"c","attachment_ids":[1,1]})),
            ["attachment_ids"]
        );
        assert_eq!(
            fields(json!({"body_md":"x","client_id":"c","attachment_ids":["1"]})),
            ["attachment_ids"]
        );
        assert!(
            parse_user_message(&json!({"body_md":"","client_id":"c","attachment_ids":[3]})).is_ok()
        );
        let big = "é".repeat(MAX_BODY_BYTES / 2 + 1);
        assert_eq!(
            parse_user_message(&json!({"body_md":big,"client_id":"c"})),
            Err(ChatError::TooLarge)
        );
        let exact = "x".repeat(MAX_BODY_BYTES);
        assert!(parse_user_message(&json!({"body_md":exact,"client_id":"c"})).is_ok());
    }
}
