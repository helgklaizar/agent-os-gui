//! The assembled bar: `BarCluster` and `DialogController`, widgets, the one-second tick, core client events, the bus.
//! Data flow: frame or widget signal -> message to the controller -> `Effect` -> bus or worker thread -> widget update.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use controllers::bar_cluster::BarCluster;
use controllers::controllers::bar::BarMsg;
use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::timezone::{Zone, system_zone};
use controllers::{Controller, CoreCommand, Effect, EventBus, MenuItem};
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreEvent};
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::bar_feed::{self, TOPICS};
use crate::bar_view::BarWidgets;
use crate::bar_worker::{BarWorker, Job, Reply};
use crate::conn_feed::ConnBridge;
use crate::dialog_host::DialogHost;

mod input;

/// What the bar receives from outside. By default there is no core connection: the clock and menu work, the rest is greyed out.
#[derive(Default)]
pub struct BarOptions {
    /// Core client for `bar.*` frames (in the product the token arrives as a descriptor).
    pub client: Option<Arc<CoreClient>>,
    /// Time zone; `None` means the system one, if `Asia/Jerusalem` is unavailable.
    pub zone: Option<Zone>,
    /// Clock (unix seconds); `None` means the system clock. Tests inject a controllable one.
    pub unix_clock: Option<fn() -> i64>,
}

fn system_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

type EventTap = Rc<dyn Fn(&CoreEvent)>;
type CommandSink = Rc<dyn Fn(CoreCommand)>;

pub(crate) struct BarRig {
    pub(crate) widgets: Rc<BarWidgets>,
    pub(crate) host: Rc<DialogHost>,
    cluster: RefCell<BarCluster>,
    /// Connection state for the "No connection" and "Core does not start" banners.
    conn: RefCell<ConnBridge>,
    dialog: RefCell<DialogController>,
    bus: EventBus,
    worker: Option<BarWorker>,
    clock: fn() -> i64,
    window: adw::ApplicationWindow,
    /// Core client event consumers other than the bar itself (the sessions column): events go through one loop and are not shared between readers.
    taps: RefCell<Vec<EventTap>>,
    /// Where core commands that the bar does not execute itself go (tree forms live in the same `DialogController`).
    commands: RefCell<Option<CommandSink>>,
    _client: Option<Arc<CoreClient>>,
}

impl BarRig {
    pub(crate) fn new(
        widgets: Rc<BarWidgets>,
        window: &adw::ApplicationWindow,
        bus: EventBus,
        options: BarOptions,
        core: Option<Arc<BlockingClient>>,
    ) -> Rc<BarRig> {
        let zone = options.zone.unwrap_or_else(|| system_zone().0);
        let (worker, replies) = match core {
            Some(c) => {
                let (w, r) = BarWorker::start(c);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let slot: Rc<RefCell<Weak<BarRig>>> = Rc::new(RefCell::new(Weak::new()));
        let send_slot = slot.clone();
        let host = DialogHost::new(Rc::new(move |msg| {
            if let Some(rig) = send_slot.borrow().upgrade() {
                rig.send_dialog(msg);
            }
        }));
        let rig = Rc::new(BarRig {
            widgets,
            host,
            cluster: RefCell::new(BarCluster::new(zone)),
            conn: RefCell::new(ConnBridge::new()),
            dialog: RefCell::new(DialogController::new()),
            bus,
            worker,
            clock: options.unix_clock.unwrap_or(system_unix),
            window: window.clone(),
            taps: RefCell::new(Vec::new()),
            commands: RefCell::new(None),
            _client: options.client.clone(),
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        rig.connect();
        rig.tick();
        if let Some(client) = options.client {
            rig.listen(&client);
        }
        if let Some(replies) = replies {
            rig.listen_replies(replies);
        }
        if let Some(w) = &rig.worker {
            w.submit(Job::Fallback);
        }
        rig
    }

    pub(crate) fn set_command_sink(&self, sink: Rc<dyn Fn(CoreCommand)>) {
        *self.commands.borrow_mut() = Some(sink);
    }

    pub(crate) fn add_tap(&self, tap: Rc<dyn Fn(&CoreEvent)>) {
        self.taps.borrow_mut().push(tap);
    }

    pub(crate) fn bar_msg(self: &Rc<Self>, msg: BarMsg) {
        let effects = self.cluster.borrow_mut().bar.update(msg, Instant::now());
        self.run(effects);
        self.refresh();
    }

    pub(crate) fn send_dialog(self: &Rc<Self>, msg: DialogMsg) {
        let effects = self.dialog.borrow_mut().update(msg, Instant::now());
        self.run(effects);
        self.refresh();
    }

    pub(crate) fn click_quota(self: &Rc<Self>, account_id: i64) {
        let effects = self
            .cluster
            .borrow_mut()
            .click_quota(account_id, Instant::now());
        self.run(effects);
    }

    fn tick(self: &Rc<Self>) {
        let effects = self
            .cluster
            .borrow_mut()
            .tick((self.clock)(), Instant::now());
        self.run(effects);
        self.refresh();
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Command(CoreCommand::FetchAbout) => {
                    if let Some(w) = &self.worker {
                        w.submit(Job::About);
                    }
                }
                Effect::Command(CoreCommand::Logout) => match &self.worker {
                    Some(w) => w.submit(Job::Logout),
                    None => {
                        gtk::glib::g_warning!("agent-shell", "logout: no connection to the core")
                    }
                },
                Effect::Command(other) => {
                    let sink = self.commands.borrow().clone();
                    match sink {
                        Some(sink) => sink(other),
                        None => gtk::glib::g_warning!(
                            "agent-shell",
                            "command {other:?} dropped: no executor"
                        ),
                    }
                }
                Effect::Persist(_) => {}
            }
        }
    }

    /// Redraw the bar and dialogs from the current controller state; quota icons get their clicks once.
    pub(crate) fn refresh(self: &Rc<Self>) {
        let view = self.cluster.borrow().view();
        let known: Vec<i64> = view.quotas.items.iter().map(|q| q.account_id).collect();
        self.widgets.apply(&view);
        self.widgets.apply_connection(&self.conn.borrow().view());
        self.apply_menu(view.bar.menu_open);
        for id in known {
            self.hook_quota(id);
        }
        self.host.apply(&self.dialog.borrow().view());
    }

    /// Show or hide the menu layer; when closed without a dialog, focus returns to the menu button.
    fn apply_menu(&self, open: bool) {
        let layer = &self.widgets.menu_layer;
        if layer.is_visible() == open {
            return;
        }
        layer.set_visible(open);
        if open {
            if let Some((_, first)) = self.widgets.menu_items.borrow().first() {
                first.grab_focus();
            }
        } else if !self.host.is_open() {
            self.widgets.menu_button.grab_focus();
        }
    }

    fn hook_quota(self: &Rc<Self>, id: i64) {
        let Some(button) = self.widgets.quota_button(id) else {
            return;
        };
        if button.has_css_class("hooked") {
            return;
        }
        button.add_css_class("hooked");
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(rig) = weak.upgrade() {
                rig.click_quota(id);
            }
        });
    }

    fn listen(self: &Rc<Self>, client: &Arc<CoreClient>) {
        for topic in TOPICS {
            if let Err(e) = client.subscribe(topic) {
                gtk::glib::g_warning!("agent-shell", "subscribe {topic}: {e}");
            }
        }
        let events = client.events();
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                let taps: Vec<_> = rig.taps.borrow().clone();
                for tap in taps {
                    tap(&event);
                }
                rig.conn.borrow_mut().on_event(&event, Instant::now());
                let routed = {
                    let mut cluster = rig.cluster.borrow_mut();
                    bar_feed::route(&mut cluster, &event, (rig.clock)(), Instant::now())
                };
                rig.run(routed.effects);
                if routed.refresh_fallback
                    && let Some(w) = &rig.worker
                {
                    w.submit(Job::Fallback);
                }
                rig.refresh();
            }
        });
    }

    fn listen_replies(self: &Rc<Self>, replies: async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                match reply {
                    Reply::Fallback(value) => {
                        let effects = {
                            let mut cluster = rig.cluster.borrow_mut();
                            bar_feed::apply_fallback(&mut cluster, value.as_deref(), Instant::now())
                        };
                        rig.run(effects);
                    }
                    Reply::About(version) => rig.send_dialog(DialogMsg::AboutVersion(version)),
                    Reply::Logout(Ok(())) => {
                        if let Some(app) = rig.window.application() {
                            app.quit();
                        }
                    }
                    Reply::Logout(Err(e)) => {
                        gtk::glib::g_warning!("agent-shell", "logout failed: {e}");
                    }
                }
                rig.refresh();
            }
        });
    }

    pub(crate) fn menu_item_button(&self, item: MenuItem) -> Option<gtk::Button> {
        self.widgets.menu_button_for(item)
    }
}
