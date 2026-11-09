//! The assembled Settings dialog: `SettingsController`, widgets, core client events, worker thread.
//! Data flow: widget signal or bus event -> message to the controller -> `Effect` -> bus or worker thread ->
//! core replies -> controller -> view.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Instant;

use controllers::controllers::settings::SettingsController;
use controllers::settings_view::{SettingsMsg, SettingsView};
use controllers::{BusEvent, Controller, CoreCommand, Effect, EventBus};
use core_client::CoreEvent;
use core_client::blocking::BlockingClient;
use gtk4 as gtk;

use crate::settings_feed::SettingsFeed;
use crate::settings_widgets::SettingsDialog;
use crate::settings_worker::SettingsWorker;

pub struct SettingsRig {
    pub dialog: Rc<SettingsDialog>,
    ctl: RefCell<SettingsController>,
    feed: RefCell<SettingsFeed>,
    bus: EventBus,
    worker: Option<SettingsWorker>,
}

/// "System" row for About: distribution name from `/etc/os-release`.
fn system_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .unwrap_or_default()
}

impl SettingsRig {
    /// `config` is the path of the shell settings file (the "Configuration" row in About).
    pub fn new(
        bus: EventBus,
        core: Option<Arc<BlockingClient>>,
        config: String,
    ) -> Rc<SettingsRig> {
        let (worker, replies) = match core {
            Some(c) => {
                let (w, r) = SettingsWorker::start(c);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let slot: Rc<RefCell<Weak<SettingsRig>>> = Rc::new(RefCell::new(Weak::new()));
        let to_rig = slot.clone();
        let dialog = SettingsDialog::new(Rc::new(move |m| {
            if let Some(rig) = to_rig.borrow().upgrade() {
                rig.send(m);
            }
        }));
        let rig = Rc::new(SettingsRig {
            dialog,
            ctl: RefCell::new(SettingsController::new()),
            feed: RefCell::new(SettingsFeed::new()),
            bus: bus.clone(),
            worker,
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        let weak = Rc::downgrade(&rig);
        bus.subscribe(move |event| {
            let Some(rig) = weak.upgrade() else { return };
            match event {
                BusEvent::OpenSettings(_) | BusEvent::ThemeChanged(_) => {
                    rig.send(SettingsMsg::Bus(event.clone()));
                }
                _ => {}
            }
        });
        rig.send(SettingsMsg::Env {
            system: system_name(),
            config,
        });
        if let Some(replies) = replies {
            let weak = Rc::downgrade(&rig);
            gtk::glib::spawn_future_local(async move {
                while let Ok(batch) = replies.recv().await {
                    let Some(rig) = weak.upgrade() else { break };
                    for reply in batch {
                        rig.send(SettingsMsg::Reply(reply));
                    }
                }
            });
        }
        rig
    }

    pub fn send(self: &Rc<Self>, msg: SettingsMsg) {
        let fx = self.ctl.borrow_mut().update(msg, Instant::now());
        self.run(fx);
        self.refresh();
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Persist(_) => {}
                Effect::Command(CoreCommand::Settings(cmd)) => match &self.worker {
                    Some(w) => w.submit(cmd),
                    None => gtk::glib::g_warning!(
                        "agent-shell",
                        "settings command dropped: no connection to the core"
                    ),
                },
                Effect::Command(_) => {}
            }
        }
    }

    fn refresh(&self) {
        let view = self.ctl.borrow().view();
        self.dialog.apply(&view);
    }

    /// Core client event (the shell's shared event loop calls it for every event).
    pub fn on_event(self: &Rc<Self>, event: &CoreEvent) {
        let msgs = self.feed.borrow_mut().route(event);
        for m in msgs {
            self.send(m);
        }
    }

    pub fn fit(&self, window_height: i32) {
        self.dialog.fit(window_height);
    }

    /// Controller view (for UI scenarios).
    pub fn view(&self) -> SettingsView {
        self.ctl.borrow().view()
    }
}
