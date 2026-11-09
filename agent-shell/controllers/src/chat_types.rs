//! Chat types as the controller sees them (docs/reference/api.md, sections 4.4 and 6): messages, permission requests, `session.<id>` frames.
//! Unknown fields are ignored; a malformed frame yields `None`.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    System,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgStatus {
    Streaming,
    Done,
    Stopped,
    Error,
    Interrupted,
}

/// A file chosen by the user for sending: path, name and size; the controller does not read the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: i64,
    pub client_id: Option<String>,
    pub role: Role,
    pub author: Option<String>,
    pub body: String,
    pub status: MsgStatus,
    pub meta: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReqStatus {
    Pending,
    Allowed,
    Denied,
    Expired,
}

/// `PermissionRequest` (docs/reference/api.md, section 6).
#[derive(Debug, Clone, PartialEq)]
pub struct PermRequest {
    pub id: i64,
    pub action: String,
    pub target: String,
    pub detail: Option<String>,
    /// `true` when the source is `observer`.
    pub observer: bool,
    pub kind: Option<String>,
    pub verdict: Option<String>,
    pub status: ReqStatus,
}

/// A session limited to what the chat needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionInfo {
    pub id: i64,
    /// Effective persona (own or inherited).
    pub persona_id: Option<i64>,
    /// The session's own persona: `None` means it inherits.
    pub own_persona_id: Option<i64>,
    pub path: Option<String>,
}

/// A frame of topic `session.<id>`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionFrame {
    Snapshot {
        session: Option<SessionInfo>,
        messages: Vec<Message>,
        open_requests: Vec<PermRequest>,
    },
    Created(Message),
    Delta {
        message_id: i64,
        offset: usize,
        text: String,
    },
    Done {
        message_id: i64,
        status: MsgStatus,
        final_len: Option<usize>,
    },
    Typing(bool),
    Request(PermRequest),
    Resolved {
        request_id: i64,
        status: ReqStatus,
    },
    Updated(SessionInfo),
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        Some(match s {
            "user" => Role::User,
            "agent" => Role::Agent,
            "system" => Role::System,
            "tool" => Role::Tool,
            _ => return None,
        })
    }
}

impl MsgStatus {
    pub fn parse(s: &str) -> Option<MsgStatus> {
        Some(match s {
            "streaming" => MsgStatus::Streaming,
            "done" => MsgStatus::Done,
            "stopped" => MsgStatus::Stopped,
            "error" => MsgStatus::Error,
            "interrupted" => MsgStatus::Interrupted,
            _ => return None,
        })
    }
}

impl ReqStatus {
    pub fn parse(s: &str) -> Option<ReqStatus> {
        Some(match s {
            "pending" => ReqStatus::Pending,
            "allowed" => ReqStatus::Allowed,
            "denied" => ReqStatus::Denied,
            "expired" | "cancelled" => ReqStatus::Expired,
            _ => return None,
        })
    }
}

impl Message {
    pub fn from_value(v: &Value) -> Option<Message> {
        Some(Message {
            id: v.get("id")?.as_i64()?,
            client_id: text(v, "client_id"),
            role: Role::parse(v.get("role")?.as_str()?)?,
            author: text(v, "author"),
            body: text(v, "body_md").unwrap_or_default(),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .and_then(MsgStatus::parse)
                .unwrap_or(MsgStatus::Done),
            meta: v.get("meta").cloned().unwrap_or(Value::Null),
            ts: v.get("ts").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

impl PermRequest {
    pub fn from_value(v: &Value) -> Option<PermRequest> {
        Some(PermRequest {
            id: v.get("id")?.as_i64()?,
            action: text(v, "action").unwrap_or_default(),
            target: text(v, "target").unwrap_or_default(),
            detail: text(v, "detail"),
            observer: text(v, "source").as_deref() == Some("observer"),
            kind: text(v, "kind"),
            verdict: text(v, "verdict"),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .and_then(ReqStatus::parse)
                .unwrap_or(ReqStatus::Pending),
        })
    }
}

impl SessionInfo {
    pub fn from_value(v: &Value) -> Option<SessionInfo> {
        let persona = |k: &str| v.get(k).and_then(Value::as_i64);
        Some(SessionInfo {
            id: v.get("id")?.as_i64()?,
            persona_id: persona("effective_persona_id").or_else(|| persona("persona_id")),
            own_persona_id: persona("persona_id"),
            path: text(v, "path"),
        })
    }
}

fn list<T>(v: &Value, key: &str, parse: fn(&Value) -> Option<T>) -> Vec<T> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse).collect())
        .unwrap_or_default()
}

/// `snapshot = true` for a frame of type `snapshot`, otherwise `data` is an event with a `kind` field.
pub fn parse_session_frame(snapshot: bool, data: &Value) -> Option<SessionFrame> {
    if snapshot {
        let mut messages = list(data, "messages", Message::from_value);
        messages.sort_by_key(|m| m.id);
        return Some(SessionFrame::Snapshot {
            session: data.get("session").and_then(SessionInfo::from_value),
            messages,
            open_requests: list(data, "open_requests", PermRequest::from_value),
        });
    }
    let int = |k: &str| data.get(k).and_then(Value::as_i64);
    let len = |k: &str| data.get(k).and_then(Value::as_u64).map(|n| n as usize);
    Some(match data.get("kind")?.as_str()? {
        "message.created" => SessionFrame::Created(Message::from_value(data.get("message")?)?),
        "message.delta" => SessionFrame::Delta {
            message_id: int("message_id")?,
            offset: len("offset")?,
            text: text(data, "text")?,
        },
        "message.done" => SessionFrame::Done {
            message_id: int("message_id")?,
            status: MsgStatus::parse(data.get("status")?.as_str()?)?,
            final_len: len("final_len"),
        },
        "typing" => SessionFrame::Typing(data.get("on")?.as_bool()?),
        "permission.request" => {
            SessionFrame::Request(PermRequest::from_value(data.get("request")?)?)
        }
        "permission.resolved" => SessionFrame::Resolved {
            request_id: int("request_id")?,
            status: ReqStatus::parse(data.get("status")?.as_str()?)?,
        },
        "session.updated" => SessionFrame::Updated(SessionInfo::from_value(data.get("session")?)?),
        _ => return None,
    })
}
