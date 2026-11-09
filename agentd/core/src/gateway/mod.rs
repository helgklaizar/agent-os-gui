//! Gateway: the single inbound bus for channel messages (see docs/reference/db.md). Intake is idempotent by
//! `(account_id, ext_id)`, the contact is matched by `handle`, rules from the `connections` page pick the session,
//! the observer isolates injections and phishing. External source text is untrusted and enters the session with a label.

mod hook;
mod model;
mod phishing;
mod routing;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{
    Audit, ChatStore, EventLevel, Events, FieldError, Finding, NewEvent, Notifier, Observer,
    SettingsStore, TreeStore,
};

pub use hook::{IncomingHook, IncomingMsg};
pub use model::{
    Direction, GatewayError, GatewayStore, Ingest, Ingested, MAX_BODY_BYTES, MAX_EXT_ID,
    NewGwMessage, Stored,
};
/// How many bytes of text the model sees and how many characters the event shows.
const MODEL_TEXT_BYTES: usize = 8 * 1024;
const EVENT_PREVIEW_CHARS: usize = 120;

pub use phishing::find_mismatch;
pub use routing::{
    Contact, channel_of, match_contact, route_for, validate as validate_connections,
};

/// Gateway dependencies.
#[derive(Clone)]
pub struct Gateway {
    pub(crate) store: Arc<dyn GatewayStore>,
    pub(crate) settings: Arc<dyn SettingsStore>,
    pub(crate) observer: Arc<dyn Observer>,
    pub(crate) audit: Audit,
    pub(crate) chat: Arc<dyn ChatStore>,
    pub(crate) tree: Arc<dyn TreeStore>,
    pub(crate) events: Events,
    pub(crate) notifier: Arc<dyn Notifier>,
    /// Subscriber to incoming messages (triggers); set once after assembly.
    pub(crate) hook: Arc<std::sync::OnceLock<Arc<dyn IncomingHook>>>,
}

fn check(i: &Ingest) -> Vec<FieldError> {
    let mut errs = Vec::new();
    if i.ext_id.is_empty() || i.ext_id.len() > MAX_EXT_ID {
        errs.push(FieldError::new("ext_id", "1 to 256 bytes"));
    }
    if i.body.trim().is_empty() || i.body.len() > MAX_BODY_BYTES {
        errs.push(FieldError::new("body", "non-empty text up to 64 KiB"));
    }
    if i.handle.len() > MAX_EXT_ID {
        errs.push(FieldError::new("handle", "at most 256 bytes"));
    }
    errs
}

fn cut(s: &str, max_bytes: usize) -> &str {
    let mut end = s.len().min(max_bytes);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl Gateway {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        store: Arc<dyn GatewayStore>,
        settings: Arc<dyn SettingsStore>,
        observer: Arc<dyn Observer>,
        audit: Audit,
        chat: Arc<dyn ChatStore>,
        tree: Arc<dyn TreeStore>,
        events: Events,
        notifier: Arc<dyn Notifier>,
    ) -> Gateway {
        Gateway {
            store,
            settings,
            observer,
            audit,
            chat,
            tree,
            events,
            notifier,
            hook: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Attaches the incoming-message subscriber; a second call changes nothing.
    pub fn set_hook(&self, hook: Arc<dyn IncomingHook>) {
        drop(self.hook.set(hook));
    }

    /// The `connections` frame is not needed for intake; the rules document is read for every message.
    async fn rules_doc(&self) -> Value {
        self.settings
            .get("connections")
            .await
            .map_or(Value::Null, |d| d.doc)
    }

    /// Channel message intake: idempotency, contact, observer isolation, routing, event.
    pub async fn ingest(&self, i: Ingest) -> Result<Ingested, GatewayError> {
        let errs = check(&i);
        if !errs.is_empty() {
            return Err(GatewayError::Invalid(errs));
        }
        let kind = self
            .store
            .account_type(i.account_id)
            .await?
            .ok_or(GatewayError::NoAccount)?;
        let channel = channel_of(&kind);
        let incoming = i.direction == Direction::In;
        let hit = if incoming {
            self.suspicious(&i.body)
        } else {
            None
        };
        let isolated = hit.is_some();
        let doc = self.rules_doc().await;
        let contact = incoming
            .then(|| match_contact(&doc, &channel, &i.handle))
            .flatten();
        let mut route = match (&contact, isolated, incoming) {
            (Some(c), false, true) => route_for(&doc, &c.name),
            _ => None,
        };
        if let Some(s) = route
            && self.tree.get_session(s).await.is_err()
        {
            route = None;
        }
        let stored = self
            .store
            .store(NewGwMessage {
                account_id: i.account_id,
                ext_id: i.ext_id.clone(),
                direction: i.direction,
                contact: contact.clone(),
                body: i.body.clone(),
                routed_session_id: route,
                isolated,
                ts: i.ts,
            })
            .await?;
        let Some(stored) = stored else {
            return Ok(Ingested {
                id: None,
                duplicate: true,
                routed_session_id: None,
                isolated: false,
                contact_id: None,
            });
        };
        if let Some((kind, detail)) = hit {
            let f = Finding {
                kind,
                severity: "high",
                ref_table: Some("gw_messages"),
                ref_id: Some(stored.id.to_string()),
                verdict: "blocked",
                detail: Some(detail),
            };
            if self.audit.finding(f).await.is_err() {
                tracing::warn!(
                    scope = "gateway",
                    event = "finding_failed",
                    "finding not recorded"
                );
            }
        }
        if incoming {
            self.announce(&i, &channel, contact.as_ref(), route, stored.id, isolated)
                .await;
        }
        Ok(Ingested {
            id: Some(stored.id),
            duplicate: false,
            routed_session_id: route,
            isolated,
            contact_id: stored.contact_id,
        })
    }

    /// Observer finding for incoming text: `(kind, detail without the message text)`.
    fn suspicious(&self, body: &str) -> Option<(&'static str, String)> {
        if let Some(hit) = self.observer.check_text(body) {
            return Some((hit.kind, format!("marker: {}", hit.rule)));
        }
        find_mismatch(body).map(|d| ("phishing", d))
    }

    /// Message in the session (labelled `untrusted`), the badge and the `msg.<channel>` event.
    async fn announce(
        &self,
        i: &Ingest,
        channel: &str,
        contact: Option<&Contact>,
        route: Option<i64>,
        id: i64,
        isolated: bool,
    ) {
        let who = contact.map_or(i.handle.as_str(), |c| c.name.as_str());
        let (level, text) = if isolated {
            (
                EventLevel::Warn,
                format!("Message from {who} isolated by the observer"),
            )
        } else {
            let preview: String = i.body.chars().take(EVENT_PREVIEW_CHARS).collect();
            (EventLevel::Info, format!("{who}: {preview}"))
        };
        if let Some(session) = route {
            self.deliver_to_session(session, channel, id, &i.body).await;
        }
        let ev = NewEvent::new("msg", channel, level, text).session(route);
        self.events.emit(ev).await;
        if !isolated && let Some(hook) = self.hook.get() {
            let msg = IncomingMsg {
                channel: channel.to_string(),
                handle: i.handle.clone(),
                contact: contact.map(|c| c.name.clone()),
                body: i.body.clone(),
                gw_id: id,
            };
            hook.on_incoming(&msg).await;
        }
    }

    pub(crate) async fn deliver_to_session(
        &self,
        session: i64,
        channel: &str,
        gw_id: i64,
        body: &str,
    ) {
        let origin = format!("message {channel}");
        let meta = json!({"kind": "gateway_message", "gw_id": gw_id, "channel": channel, "untrusted": true});
        self.deliver(session, &origin, meta, body).await;
    }

    /// MCP server notification into a trigger session: untrusted text with the source "MCP server notification".
    pub(crate) async fn deliver_notice(&self, session: i64, server: &str, body: &str) {
        let origin = format!("MCP server notification {server}");
        let meta = json!({"kind": "mcp_notice", "server": server, "untrusted": true});
        self.deliver(session, &origin, meta, body).await;
    }

    async fn deliver(&self, session: i64, origin: &str, meta: serde_json::Value, body: &str) {
        let doc = json!({"action": null, "code": "gateway", "ok": true, "untrusted": true,
                         "origin": origin, "output": cut(body, MODEL_TEXT_BYTES)});
        match self
            .chat
            .create_tool(session, format!("[tool_result {doc}]"), meta)
            .await
        {
            Ok(msg) => {
                self.notifier.publish_event(
                    &format!("session.{session}"),
                    json!({"kind": "message.created", "message": msg}),
                );
                if self.tree.bump_unread(session).await.is_err() {
                    tracing::debug!(
                        scope = "gateway",
                        event = "unread_skipped",
                        "session vanished"
                    );
                }
            }
            Err(_) => tracing::debug!(
                scope = "gateway",
                event = "route_skipped",
                "session vanished"
            ),
        }
    }
}
