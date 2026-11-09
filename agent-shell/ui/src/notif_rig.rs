//! The assembled "Notifications" column: `NotificationsController`, widgets, core client events, worker thread.
//! Data flow: `events` frame or widget signal -> message to the controller -> `Effect` -> bus or worker thread -> views.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use controllers::controllers::notifications::{NotifMsg, NotificationsController};
use controllers::timezone::Zone;
use controllers::{BusEvent, Controller, CoreCommand, Effect, EventBus};
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreEvent};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::chat_feed::resubscribe;
use crate::notif_feed::{self, TOPICS, apply_subtypes};
use crate::notif_widgets::NotifWidgets;
use crate::notif_worker::{Job, NotifWorker, Reply};

pub(crate) struct NotifRig {
    pub(crate) widgets: Rc<NotifWidgets>,
    ctl: RefCell<NotificationsController>,
    bus: EventBus,
    worker: Option<NotifWorker>,
    client: Option<Arc<CoreClient>>,
    zone: Zone,
    /// Unread badge on the column icon.
    badge: gtk::Label,
    /// Window title: `(N) Agent OS`.
    on_title: Rc<dyn Fn(&str)>,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

impl NotifRig {
    pub(crate) fn new(
        body: &gtk::Box,
        badge: gtk::Label,
        bus: EventBus,
        core: Option<Arc<BlockingClient>>,
        client: Option<&Arc<CoreClient>>,
        zone: Zone,
        on_title: Rc<dyn Fn(&str)>,
    ) -> Rc<NotifRig> {
        let (worker, replies) = match core {
            Some(c) => {
                let (w, r) = NotifWorker::start(c);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let slot: Rc<RefCell<std::rc::Weak<NotifRig>>> =
            Rc::new(RefCell::new(std::rc::Weak::new()));
        let send_slot = slot.clone();
        let widgets = NotifWidgets::new(Rc::new(move |msg| {
            if let Some(rig) = send_slot.borrow().upgrade() {
                rig.send(msg);
            }
        }));
        body.append(&widgets.root);
        let rig = Rc::new(NotifRig {
            widgets,
            ctl: RefCell::new(NotificationsController::new()),
            bus: bus.clone(),
            worker,
            client: client.cloned(),
            zone,
            badge,
            on_title,
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        let weak = Rc::downgrade(&rig);
        bus.subscribe(move |event| {
            if let (Some(rig), BusEvent::OpenSession { .. }) = (weak.upgrade(), event) {
                rig.send(NotifMsg::Bus(event.clone()));
            }
        });
        rig.send(NotifMsg::UtcOffset(rig.zone.offset_at(now_unix())));
        rig.job(Job::Subtypes);
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

    pub(crate) fn send(self: &Rc<Self>, msg: NotifMsg) {
        let effects = self.ctl.borrow_mut().update(msg, Instant::now());
        self.run(effects);
        self.refresh();
    }

    pub(crate) fn collapsed(self: &Rc<Self>, collapsed: bool) {
        self.send(NotifMsg::Collapsed(collapsed));
    }

    fn job(&self, job: Job) {
        match &self.worker {
            Some(w) => w.submit(job),
            None => gtk::glib::g_warning!(
                "agent-shell",
                "notifications job dropped: no connection to the core"
            ),
        }
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Persist(_) => {}
                Effect::Command(CoreCommand::FetchEventSubtypes) => self.job(Job::Subtypes),
                Effect::Command(CoreCommand::ResyncEvents) => {
                    if let Some(c) = &self.client
                        && let Err(e) = resubscribe(c, "events")
                    {
                        gtk::glib::g_warning!("agent-shell", "resync events: {e}");
                    }
                }
                Effect::Command(cmd) => self.job(Job::Exec(cmd)),
            }
        }
    }

    pub(crate) fn refresh(self: &Rc<Self>) {
        let view = self.ctl.borrow().view();
        self.widgets.apply(&view);
        match &view.all.badge.text {
            Some(text) => {
                self.badge.set_text(text);
                self.badge.set_visible(true);
            }
            None => self.badge.set_visible(false),
        }
        a11y::set_name(&self.badge, &view.all.badge.a11y_name);
        (self.on_title)(&view.title);
    }

    /// Core client event (the shell's shared event loop calls it for every event).
    pub(crate) fn on_event(self: &Rc<Self>, event: &CoreEvent) {
        let routed = notif_feed::route(&mut self.ctl.borrow_mut(), event, Instant::now());
        self.run(routed.effects);
        if routed.refresh_subtypes {
            self.job(Job::Subtypes);
            self.send(NotifMsg::UtcOffset(self.zone.offset_at(now_unix())));
        }
        self.refresh();
    }

    fn listen_replies(self: &Rc<Self>, replies: async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                match reply {
                    Reply::Subtypes(list) => {
                        let fx = apply_subtypes(&mut rig.ctl.borrow_mut(), &list, Instant::now());
                        rig.run(fx);
                    }
                    Reply::Failed(why) => {
                        gtk::glib::g_warning!("agent-shell", "notifications command failed: {why}")
                    }
                }
                rig.refresh();
            }
        });
    }

    /// Current feed view (for UI scenarios).
    pub(crate) fn view(&self) -> controllers::notif_view::NotifView {
        self.ctl.borrow().view()
    }
}
