//! The assembled sessions column: `SessionTreeController`, widgets, core client events, worker thread.
//! Data flow: `sessions` frame or widget signal -> message to the controller -> `Effect` -> bus or worker thread -> views.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::{BusEvent, Controller, Effect, EventBus, FocusTarget, TreeButton};
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreEvent};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::tree_feed::{self, TOPICS};
use crate::tree_widgets::TreeWidgets;
use crate::tree_worker::{Job, Reply, TreeWorker};

pub(crate) struct TreeRig {
    pub(crate) widgets: Rc<TreeWidgets>,
    tree: RefCell<SessionTreeController>,
    bus: EventBus,
    worker: Option<TreeWorker>,
    /// Unread badge on the collapsed column's icon.
    badge: gtk::Label,
}

impl TreeRig {
    pub(crate) fn new(
        body: &gtk::Box,
        badge: gtk::Label,
        bus: EventBus,
        core: Option<Arc<BlockingClient>>,
        client: Option<&Arc<CoreClient>>,
    ) -> Rc<TreeRig> {
        let (worker, replies) = match core {
            Some(c) => {
                let (w, r) = TreeWorker::start(c);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let slot: Rc<RefCell<std::rc::Weak<TreeRig>>> = Rc::new(RefCell::new(std::rc::Weak::new()));
        let send_slot = slot.clone();
        let widgets = TreeWidgets::new(Rc::new(move |msg| {
            if let Some(rig) = send_slot.borrow().upgrade() {
                rig.send(msg);
            }
        }));
        body.append(&widgets.root);
        let rig = Rc::new(TreeRig {
            widgets,
            tree: RefCell::new(SessionTreeController::new()),
            bus,
            worker,
            badge,
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        // the session was opened not from the tree (a feed card): select it and reset the badge
        let weak = Rc::downgrade(&rig);
        rig.bus.subscribe(move |event| {
            if let (Some(rig), BusEvent::OpenSession { session_id, .. }) = (weak.upgrade(), event) {
                rig.send(TreeMsg::Opened(*session_id));
            }
        });
        rig.refresh();
        if let Some(client) = client {
            for topic in TOPICS {
                if let Err(e) = client.subscribe(topic) {
                    gtk::glib::g_warning!("agent-shell", "subscribe {topic}: {e}");
                }
            }
        }
        if let Some(replies) = replies {
            rig.listen_replies(replies);
        }
        rig
    }

    pub(crate) fn send(self: &Rc<Self>, msg: TreeMsg) {
        let move_focus = matches!(msg, TreeMsg::Key(_) | TreeMsg::Focus(_));
        let effects = self.tree.borrow_mut().update(msg, Instant::now());
        self.run(effects);
        self.refresh();
        // code moves focus only after arrows and a click; ordinary frames do not touch it
        if move_focus {
            self.widgets.focus_node(self.tree.borrow().view().focus);
        }
    }

    pub(crate) fn collapsed(self: &Rc<Self>, collapsed: bool) {
        self.send(TreeMsg::Collapsed(collapsed));
    }

    /// Core command from a dialog: executed by the tree worker thread.
    pub(crate) fn submit(&self, command: controllers::CoreCommand) {
        match &self.worker {
            Some(w) => w.submit(Job::Exec(command)),
            None => gtk::glib::g_warning!(
                "agent-shell",
                "tree command dropped: no connection to the core"
            ),
        }
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Command(cmd) => match &self.worker {
                    Some(w) => w.submit(Job::Exec(cmd)),
                    None => gtk::glib::g_warning!(
                        "agent-shell",
                        "tree command dropped: no connection to the core"
                    ),
                },
                Effect::Persist(_) => {}
            }
        }
    }

    pub(crate) fn refresh(self: &Rc<Self>) {
        let view = self.tree.borrow().view();
        self.widgets.apply(&view);
        match view.strip.as_ref().and_then(|s| s.badge.clone()) {
            Some(text) => {
                self.badge.set_text(&text);
                self.badge.set_visible(true);
            }
            None => self.badge.set_visible(false),
        }
        if let Some(strip) = &view.strip {
            a11y::set_name(&self.badge, &strip.a11y_name);
        }
    }

    /// Core client event (the shell's shared event loop calls it for every event).
    pub(crate) fn on_event(self: &Rc<Self>, event: &CoreEvent) {
        let routed = tree_feed::route(&mut self.tree.borrow_mut(), event, Instant::now());
        self.run(routed.effects);
        if routed.refresh_personas
            && let Some(w) = &self.worker
        {
            w.submit(Job::Personas);
        }
        self.refresh();
    }

    fn listen_replies(self: &Rc<Self>, replies: async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                match reply {
                    Reply::Personas(list) => {
                        let fx = tree_feed::apply_personas(
                            &mut rig.tree.borrow_mut(),
                            &list,
                            Instant::now(),
                        );
                        rig.run(fx);
                    }
                    Reply::Failed(why) => {
                        gtk::glib::g_warning!("agent-shell", "tree command failed: {why}")
                    }
                }
                rig.refresh();
            }
        });
    }

    /// Return focus to the calling element after a dialog closes.
    pub(crate) fn restore_focus(&self, target: FocusTarget) {
        match target {
            FocusTarget::Node(node) => self.widgets.focus_node(Some(node)),
            FocusTarget::TreeButton(TreeButton::NewSession) => {
                self.widgets.new_session.grab_focus();
            }
            FocusTarget::TreeButton(TreeButton::NewFolder) => {
                self.widgets.new_folder.grab_focus();
            }
            FocusTarget::MenuItem(_) => {}
        }
    }

    /// Current tree view (for UI scenarios).
    pub(crate) fn view(&self) -> controllers::tree_view::TreeView {
        self.tree.borrow().view()
    }
}
