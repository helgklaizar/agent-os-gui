//! The assembled "Chat" column: `ChatController`, widgets, core client events, worker thread.
//! Data flow: `session.<id>` frame or widget signal -> message to the controller -> `Effect` -> bus or worker thread -> view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{BusEvent, Controller, CoreCommand, Effect, EventBus};
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreEvent};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::chat_feed::{self, apply_permissions, apply_personas, resubscribe, topic};
use crate::chat_widgets::ChatWidgets;
use crate::chat_worker::{ChatWorker, Job, Reply};

pub(crate) struct ChatRig {
    pub(crate) widgets: Rc<ChatWidgets>,
    chat: RefCell<ChatController>,
    bus: EventBus,
    worker: Option<ChatWorker>,
    client: Option<Arc<CoreClient>>,
    active: Cell<Option<i64>>,
    dialog_open: Cell<bool>,
}

impl ChatRig {
    pub(crate) fn new(
        body: &gtk::Box,
        bus: EventBus,
        core: Option<Arc<BlockingClient>>,
        client: Option<&Arc<CoreClient>>,
    ) -> Rc<ChatRig> {
        let (worker, replies) = match (core, client) {
            (Some(c), Some(cl)) => {
                let (w, r) = ChatWorker::start(c, cl.clone());
                (Some(w), Some(r))
            }
            _ => (None, None),
        };
        let slot: Rc<RefCell<std::rc::Weak<ChatRig>>> = Rc::new(RefCell::new(std::rc::Weak::new()));
        let send_slot = slot.clone();
        let widgets = ChatWidgets::new(Rc::new(move |msg| {
            if let Some(rig) = send_slot.borrow().upgrade() {
                rig.send(msg);
            }
        }));
        body.append(&widgets.root);
        let rig = Rc::new(ChatRig {
            widgets,
            chat: RefCell::new(ChatController::new()),
            bus: bus.clone(),
            worker,
            client: client.cloned(),
            active: Cell::new(None),
            dialog_open: Cell::new(false),
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        let weak = Rc::downgrade(&rig);
        bus.subscribe(move |event| {
            if let (Some(rig), BusEvent::OpenSession { session_id, .. }) = (weak.upgrade(), event) {
                rig.select(Some(*session_id));
            }
        });
        rig.send(ChatMsg::Select(None));
        if let Some(replies) = replies {
            rig.listen_replies(replies);
        }
        rig
    }

    /// The active session changed: resubscribe to `session.<id>` and select it in the controller.
    fn select(self: &Rc<Self>, id: Option<i64>) {
        let old = self.active.replace(id);
        if old == id {
            return;
        }
        if let Some(c) = &self.client {
            if let Some(o) = old
                && let Err(e) = c.unsubscribe(&topic(o))
            {
                gtk::glib::g_warning!("agent-shell", "unsubscribe session: {e}");
            }
            if let Some(n) = id
                && let Err(e) = c.subscribe(&topic(n))
            {
                gtk::glib::g_warning!("agent-shell", "subscribe session: {e}");
            }
        }
        self.send(ChatMsg::Select(id));
    }

    pub(crate) fn send(self: &Rc<Self>, msg: ChatMsg) {
        let effects = self.chat.borrow_mut().update(msg, Instant::now());
        self.run(effects);
        self.refresh();
    }

    fn job(&self, job: Job) {
        match &self.worker {
            Some(w) => w.submit(job),
            None => {
                gtk::glib::g_warning!("agent-shell", "chat job dropped: no connection to the core")
            }
        }
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(e) => self.bus.publish(e),
                Effect::Persist(_) => {}
                Effect::Command(CoreCommand::SendMessage {
                    session_id,
                    client_id,
                    body_md,
                    attachments,
                }) => self.job(Job::Send {
                    session_id,
                    client_id,
                    body_md,
                    attachments,
                }),
                Effect::Command(CoreCommand::ListPersonas) => self.job(Job::Personas),
                Effect::Command(CoreCommand::ResyncSession { session_id }) => {
                    if let Some(c) = &self.client
                        && let Err(e) = resubscribe(c, &topic(session_id))
                    {
                        gtk::glib::g_warning!("agent-shell", "resync session: {e}");
                    }
                }
                Effect::Command(cmd) => self.job(Job::Exec(cmd)),
            }
        }
    }

    pub(crate) fn refresh(self: &Rc<Self>) {
        let view = self.chat.borrow().view();
        self.widgets.apply(&view);
        if view.want_path_dialog && !self.dialog_open.replace(true) {
            self.open_dialog(true);
        } else if view.want_attach_dialog && !self.dialog_open.replace(true) {
            self.open_dialog(false);
        }
    }

    /// GTK file dialog: session folder or attachment; the result goes to the controller.
    fn open_dialog(self: &Rc<Self>, folder: bool) {
        let dialog = gtk::FileDialog::new();
        let parent = self.widgets.root.root().and_downcast::<gtk::Window>();
        let weak = Rc::downgrade(self);
        let done = move |path: Option<std::path::PathBuf>| {
            let Some(rig) = weak.upgrade() else { return };
            rig.dialog_open.set(false);
            let msg = match (folder, path) {
                (true, p) => ChatMsg::PickPath(p.map(|p| p.to_string_lossy().into_owned())),
                (false, None) => ChatMsg::AttachDialogClosed,
                (false, Some(p)) => {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    ChatMsg::AttachPicked {
                        path: p.to_string_lossy().into_owned(),
                        name,
                        size,
                    }
                }
            };
            rig.send(msg);
        };
        if folder {
            dialog.select_folder(parent.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
                done(r.ok().and_then(|f| f.path()));
            });
        } else {
            dialog.open(parent.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
                done(r.ok().and_then(|f| f.path()));
            });
        }
    }

    /// Core client event (the shell's shared event loop calls it for every event).
    pub(crate) fn on_event(self: &Rc<Self>, event: &CoreEvent) {
        let routed = chat_feed::route(&mut self.chat.borrow_mut(), event, Instant::now());
        self.run(routed.effects);
        if routed.refresh {
            self.job(Job::Personas);
            self.job(Job::Permissions);
        }
        if routed.deleted.is_some() && routed.deleted == self.active.get() {
            self.select(None);
        }
        self.refresh();
    }

    fn listen_replies(self: &Rc<Self>, replies: async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                let now = Instant::now();
                let effects = match reply {
                    Reply::Personas(list) => apply_personas(&mut rig.chat.borrow_mut(), &list, now),
                    Reply::Permissions(rows) => {
                        apply_permissions(&mut rig.chat.borrow_mut(), &rows, now)
                    }
                    Reply::Decision {
                        request_id,
                        outcome,
                    } => rig.on_decision(request_id, outcome),
                    Reply::SendFailed { client_id, why } => {
                        gtk::glib::g_warning!("agent-shell", "chat send failed: {why}");
                        rig.chat
                            .borrow_mut()
                            .update(ChatMsg::SendFailed { client_id }, now)
                    }
                    Reply::Failed(why) => {
                        gtk::glib::g_warning!("agent-shell", "chat command failed: {why}");
                        Vec::new()
                    }
                };
                rig.run(effects);
                rig.refresh();
            }
        });
    }

    fn on_decision(
        &self,
        request_id: i64,
        outcome: crate::chat_exec::DecisionOutcome,
    ) -> Vec<Effect> {
        use crate::chat_exec::DecisionOutcome as O;
        let msg = match outcome {
            O::Done => return Vec::new(),
            O::Rejected(status) => ChatMsg::DecisionRejected { request_id, status },
            O::Failed => ChatMsg::DecisionFailed { request_id },
        };
        self.chat.borrow_mut().update(msg, Instant::now())
    }

    /// Current chat view (for UI scenarios).
    pub(crate) fn view(&self) -> controllers::chat_view::ChatView {
        self.chat.borrow().view()
    }
}
