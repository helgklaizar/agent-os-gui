//! Building chat bubbles from messages: the "Agent · Persona" label, time, Markdown.

use crate::chat_attach::message_previews;
use crate::chat_types::{Message, MsgStatus, Role};
use crate::chat_view::{Bubble, ChatItem, Delivery, ErrorCard};
use crate::clock_fmt::format_time;
use crate::i18n::{tr, tr_args};
use crate::markdown::render;
use crate::tree_types::PersonaItem;

/// Author label: the user is "You", the agent is "Agent · Persona" (the session's persona from `GET`, not from the message text).
pub fn author_line(role: Role, author: Option<&str>, persona: Option<&PersonaItem>) -> String {
    match role {
        Role::User => tr("chat-author-you"),
        Role::Agent => match (persona, author) {
            (Some(p), _) => tr_args("chat-author-agent", &[("persona", &p.name)]),
            (None, Some(a)) if !a.is_empty() => a.to_string(),
            _ => tr("chat-author-agent-plain"),
        },
        Role::System => tr("chat-author-system"),
        Role::Tool => tr("chat-author-tool"),
    }
}

/// `HH:MM` of the message time.
pub fn short_time(ts: i64, offset: i32) -> String {
    format_time(ts, offset).chars().take(5).collect()
}

pub fn bubble(m: &Message, persona: Option<&PersonaItem>, offset: i32) -> Bubble {
    Bubble {
        id: Some(m.id),
        client_id: m.client_id.clone(),
        mine: m.role == Role::User,
        author_line: author_line(m.role, m.author.as_deref(), persona),
        time: short_time(m.ts, offset),
        blocks: render(&m.body),
        streaming: m.status == MsgStatus::Streaming,
        note: match m.status {
            MsgStatus::Stopped => Some(tr("chat-stopped")),
            MsgStatus::Interrupted => Some(tr("chat-interrupted")),
            _ => None,
        },
        delivery: Delivery::Sent,
        attachments: message_previews(&m.meta),
    }
}

/// Stream item: a bubble or an error card (status `error`).
pub fn item(m: &Message, persona: Option<&PersonaItem>, offset: i32) -> ChatItem {
    if m.status == MsgStatus::Error && m.role == Role::Agent {
        return ChatItem::Error(ErrorCard {
            message_id: m.id,
            title: tr("chat-error-title"),
            text: m.body.clone(),
        });
    }
    ChatItem::Bubble(bubble(m, persona, offset))
}
