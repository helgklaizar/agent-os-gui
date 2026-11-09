//! `NotificationsController`: notification feed. A snapshot at start plus deltas from WS, filter subtypes
//! from the core registry, the `All → kind → subtype` filter, unread counters, "Open chat", toast, window title.

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::controller::{Controller, CoreCommand, Effect};
use crate::notif_model::Model;
use crate::notif_render;
use crate::notif_types::{EventCounts, EventItem, EventKind, EventsFilter, SubtypeItem, tab_of};
use crate::notif_view::{NotifView, Selection};
use crate::tree_types::{SessionItem, TreeFrame};
use crate::types::Column;

#[derive(Debug, Clone, PartialEq)]
pub enum NotifMsg {
    /// Snapshot of the `events` topic (replaces everything after each (re)subscription).
    Snapshot {
        items: Vec<EventItem>,
        counts: EventCounts,
    },
    /// `event` frame: an event was created.
    Created(EventItem),
    /// Delta `{ read: [id…] }`.
    Read(Vec<i64>),
    /// Subtype registry (`GET /events/subtypes`).
    Subtypes(Vec<SubtypeItem>),
    /// Sessions for card captions (the `sessions` snapshot) and their changes.
    Sessions(Vec<SessionItem>),
    Tree(TreeFrame),
    SelectAll,
    SelectKind(EventKind),
    SelectSub(EventKind, String),
    /// Card button ("Open chat" or "OK").
    Open(i64),
    /// Mark as read by filter.
    MarkRead(EventsFilter),
    /// Column collapsed or expanded (the shell decides the layout).
    Collapsed(bool),
    /// Click on a kind icon on the collapsed strip.
    StripClick(EventKind),
    /// User's UTC offset in seconds (time on cards).
    UtcOffset(i32),
    Bus(BusEvent),
}

#[derive(Debug, Default)]
pub struct NotificationsController {
    model: Model,
}

impl NotificationsController {
    pub fn new() -> Self {
        Self::default()
    }

    /// All session events are read: locally and in the core (`POST /events/read` with `session_id`).
    fn read_session(&mut self, session_id: i64, fx: &mut Vec<Effect>) {
        let filter = EventsFilter {
            session_id: Some(session_id),
            ..EventsFilter::default()
        };
        self.model.mark_filter(&filter);
        fx.push(Effect::Command(CoreCommand::MarkEventsRead(filter)));
    }

    /// New event. For the open session it is read at once; an AI reply in another session shows a toast
    /// (the session badge in the list is driven by the tree from the `unread` frame). A repeated `id` does nothing.
    fn created(&mut self, e: EventItem, fx: &mut Vec<Effect>) {
        let (id, session, unread, ai) = (
            e.id,
            e.session_id,
            e.unread,
            e.kind == EventKind::Ai.as_str(),
        );
        let toast = (ai && unread)
            .then_some(session)
            .flatten()
            .filter(|s| self.model.active != Some(*s))
            .map(|s| (s, notif_render::toast_text(&self.model, &e)));
        if !self.model.created(e) {
            return;
        }
        if unread && session.is_some() && session == self.model.active {
            self.model.read(&[id]);
            fx.push(Effect::Command(CoreCommand::MarkEventRead { id }));
        }
        if let Some((session_id, text)) = toast {
            fx.push(Effect::Publish(BusEvent::ShowToast { session_id, text }));
        }
    }

    /// Card button: an event with a session opens it and the tab from `Event.tab`; without a session it only marks as read.
    fn open(&mut self, id: i64, fx: &mut Vec<Effect>) {
        let Some(event) = self.model.event(id) else {
            return;
        };
        let (session, tab, unread) = (event.session_id, tab_of(&event.tab), event.unread);
        match session {
            Some(session_id) => {
                self.model.active = Some(session_id);
                self.read_session(session_id, fx);
                fx.push(Effect::Publish(BusEvent::OpenSession { session_id, tab }));
            }
            None if unread => {
                self.model.read(&[id]);
                fx.push(Effect::Command(CoreCommand::MarkEventRead { id }));
            }
            None => {}
        }
    }

    fn tree_frame(&mut self, frame: TreeFrame) {
        match frame {
            TreeFrame::SessionCreated(s) | TreeFrame::SessionUpdated(s) => {
                self.model.sessions.insert(s.id, s);
            }
            TreeFrame::SessionDeleted(id) => {
                let gone: Vec<i64> = self
                    .model
                    .sessions
                    .values()
                    .filter(|s| s.id == id || s.parent_id == Some(id))
                    .map(|s| s.id)
                    .collect();
                for id in gone {
                    self.model.sessions.remove(&id);
                    // the event lives without a session (the core nulls session_id, see docs/reference/db.md)
                    for e in self
                        .model
                        .events
                        .iter_mut()
                        .filter(|e| e.session_id == Some(id))
                    {
                        e.session_id = None;
                    }
                }
            }
            TreeFrame::FolderCreated(_)
            | TreeFrame::FolderUpdated(_)
            | TreeFrame::FolderDeleted(_)
            | TreeFrame::Unread { .. } => {}
        }
    }
}

impl Controller for NotificationsController {
    type Msg = NotifMsg;
    type View = NotifView;

    fn update(&mut self, msg: NotifMsg, _now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match msg {
            NotifMsg::Snapshot { items, counts } => self.model.snapshot(items, counts),
            NotifMsg::Created(e) => self.created(e, &mut fx),
            NotifMsg::Read(ids) => {
                if self.model.read(&ids) {
                    fx.push(Effect::Command(CoreCommand::ResyncEvents));
                }
            }
            NotifMsg::Subtypes(list) => self.model.registry = list,
            NotifMsg::Sessions(list) => {
                self.model.sessions = list.into_iter().map(|s| (s.id, s)).collect();
            }
            NotifMsg::Tree(frame) => self.tree_frame(frame),
            NotifMsg::SelectAll => self.model.select_all(),
            NotifMsg::SelectKind(kind) => self.model.select_kind(kind),
            NotifMsg::SelectSub(kind, sub) => self.model.select_sub(kind, sub),
            NotifMsg::MarkRead(filter) => {
                self.model.mark_filter(&filter);
                fx.push(Effect::Command(CoreCommand::MarkEventsRead(filter)));
            }
            NotifMsg::Collapsed(c) => self.model.collapsed = c,
            NotifMsg::UtcOffset(secs) => self.model.utc_offset = secs,
            NotifMsg::StripClick(kind) => {
                // on the strip a click selects the kind and a second click does not clear it: the column expands with this filter
                if !matches!(&self.model.selection, Selection::Kind(k) | Selection::Sub(k, _) if *k == kind)
                {
                    self.model.selection = Selection::Kind(kind);
                }
                fx.push(Effect::Publish(BusEvent::ExpandColumn(
                    Column::Notifications,
                )));
            }
            NotifMsg::Open(id) => self.open(id, &mut fx),
            NotifMsg::Bus(BusEvent::OpenSession { session_id, .. }) => {
                self.model.active = Some(session_id);
                if self.model.has_unread_of(session_id) {
                    self.read_session(session_id, &mut fx);
                }
            }
            NotifMsg::Bus(_) => {}
        }
        fx
    }

    fn view(&self) -> NotifView {
        notif_render::build(&self.model)
    }
}
