//! The assembled "Tools" column: `ToolsController` and three tabs, core client events, worker thread.
//! Data flow: frame or widget signal -> message to the controller -> `Effect` -> bus or worker thread -> views.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use controllers::browser_view::BrowserMsg;
use controllers::controllers::tools::{ToolsController, ToolsMsg};
use controllers::ss_view::SsMsg;
use controllers::terminal_view::TermMsg;
use controllers::tree_types::parse_personas;
use controllers::{BusEvent, Controller, Effect, EventBus, Tab};
use core_client::blocking::BlockingClient;
use core_client::{ConnState, CoreEvent};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::browser_pane::BrowserPane;
use crate::ss_pane::SettingsPane;
use crate::terminal_pane::TerminalPane;
use crate::tools_feed::ToolsCtls;
use crate::tools_widgets::ToolsWidgets;
use crate::tools_worker::{Job, Reply, ToolsWorker};

pub struct ToolsOptions {
    /// Root of the WebKit profiles (`sessions/<id>`, `agent/<id>`).
    pub data_dir: PathBuf,
    pub home: PathBuf,
}

pub(crate) struct ToolsRig {
    pub(crate) widgets: Rc<ToolsWidgets>,
    pub(crate) browser: Rc<BrowserPane>,
    pub(crate) terminal: Rc<TerminalPane>,
    pub(crate) settings: Rc<SettingsPane>,
    tabs: RefCell<ToolsController>,
    ctls: RefCell<ToolsCtls>,
    bus: EventBus,
    worker: Option<ToolsWorker>,
    /// The column is visible only while a session is open.
    on_visible: Rc<dyn Fn(bool)>,
}

impl ToolsRig {
    pub(crate) fn new(
        body: &gtk::Box,
        bus: EventBus,
        core: Option<Arc<BlockingClient>>,
        options: ToolsOptions,
        on_move: Rc<dyn Fn(i64)>,
        on_visible: Rc<dyn Fn(bool)>,
    ) -> Rc<ToolsRig> {
        let (worker, replies) = match core {
            Some(c) => {
                let (w, r) = ToolsWorker::start(c);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let slot: Rc<RefCell<std::rc::Weak<ToolsRig>>> =
            Rc::new(RefCell::new(std::rc::Weak::new()));
        let (s1, s2, s3, s4) = (slot.clone(), slot.clone(), slot.clone(), slot.clone());
        let widgets = ToolsWidgets::new(Rc::new(move |tab: Tab| {
            if let Some(rig) = s1.borrow().upgrade() {
                rig.send_tab(ToolsMsg::Select(tab));
            }
        }));
        let browser = BrowserPane::new(
            options.data_dir,
            Rc::new(move |m| {
                if let Some(rig) = s2.borrow().upgrade() {
                    rig.send_browser(m);
                }
            }),
        );
        let terminal = TerminalPane::new(Rc::new(move |m| {
            if let Some(rig) = s3.borrow().upgrade() {
                rig.send_term(m);
            }
        }));
        let settings = SettingsPane::new(
            Rc::new(move |m| {
                if let Some(rig) = s4.borrow().upgrade() {
                    rig.send_ss(m);
                }
            }),
            on_move,
        );
        widgets.add_page(Tab::Browser, &browser.root);
        widgets.add_page(Tab::Terminal, &terminal.root);
        widgets.add_page(Tab::SessionSettings, &settings.root);
        body.append(&widgets.root);
        let rig = Rc::new(ToolsRig {
            widgets,
            browser,
            terminal,
            settings,
            tabs: RefCell::new(ToolsController::new()),
            ctls: RefCell::new(ToolsCtls::new(options.home)),
            bus: bus.clone(),
            worker,
            on_visible,
        });
        *slot.borrow_mut() = Rc::downgrade(&rig);
        let weak = Rc::downgrade(&rig);
        bus.subscribe(move |event| {
            if let (Some(rig), BusEvent::OpenSession { session_id, .. }) = (weak.upgrade(), event) {
                rig.opened(*session_id, event);
            }
        });
        rig.job(Job::Personas);
        if let Some(replies) = replies {
            rig.listen_replies(replies);
        }
        rig.refresh();
        rig
    }

    fn opened(self: &Rc<Self>, session_id: i64, event: &BusEvent) {
        let fx = self
            .tabs
            .borrow_mut()
            .update(ToolsMsg::Bus(event.clone()), Instant::now());
        self.run(fx);
        let fx = self
            .ctls
            .borrow_mut()
            .open(Some(session_id), Instant::now());
        self.run(fx);
        self.refresh();
    }

    pub(crate) fn send_tab(self: &Rc<Self>, msg: ToolsMsg) {
        let fx = self.tabs.borrow_mut().update(msg, Instant::now());
        self.run(fx);
        self.refresh();
    }

    pub(crate) fn send_browser(self: &Rc<Self>, msg: BrowserMsg) {
        let fx = self.ctls.borrow_mut().browser.update(msg, Instant::now());
        self.run(fx);
        self.refresh();
    }

    pub(crate) fn send_term(self: &Rc<Self>, msg: TermMsg) {
        let fx = self.ctls.borrow_mut().terminal.update(msg, Instant::now());
        self.run(fx);
        self.refresh();
    }

    pub(crate) fn send_ss(self: &Rc<Self>, msg: SsMsg) {
        let fx = self.ctls.borrow_mut().settings.update(msg, Instant::now());
        self.run(fx);
        self.refresh();
    }

    pub(crate) fn collapsed(self: &Rc<Self>, collapsed: bool) {
        self.send_tab(ToolsMsg::Collapsed(collapsed));
    }

    fn job(&self, job: Job) {
        match &self.worker {
            Some(w) => w.submit(job),
            None => gtk::glib::g_warning!(
                "agent-shell",
                "tools job dropped: no connection to the core"
            ),
        }
    }

    fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Persist(_) => {}
                Effect::Command(cmd) => self.job(Job::Exec(cmd)),
            }
        }
    }

    pub(crate) fn refresh(self: &Rc<Self>) {
        let tabs = self.tabs.borrow().view();
        self.widgets.apply(&tabs);
        (self.on_visible)(tabs.visible);
        // views are taken in advance: widgets may send messages to controllers while applying
        let (b, t, s) = {
            let c = self.ctls.borrow();
            (c.browser.view(), c.terminal.view(), c.settings.view())
        };
        self.browser.apply(&b);
        self.terminal
            .apply(&t, tabs.visible && tabs.active == Tab::Terminal);
        self.settings.apply(&s);
    }

    /// Core client event (the shell's shared event loop calls it for every event).
    pub(crate) fn on_event(self: &Rc<Self>, event: &CoreEvent) {
        if matches!(event, CoreEvent::State(ConnState::Connected)) {
            self.job(Job::Personas);
        }
        let fx = self.ctls.borrow_mut().route(event, Instant::now());
        self.run(fx);
        // the session was deleted: settings closed by themselves, the column hides
        let gone = self.tabs.borrow().view().session.is_some()
            && self.ctls.borrow().settings.view().session_id.is_none();
        if gone {
            let fx = self.ctls.borrow_mut().open(None, Instant::now());
            self.run(fx);
            self.send_tab(ToolsMsg::SessionClosed);
        }
        self.refresh();
    }

    fn listen_replies(self: &Rc<Self>, replies: async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(rig) = weak.upgrade() else { break };
                match reply {
                    Reply::Tools(list) => {
                        for r in list {
                            let fx = rig.ctls.borrow_mut().reply(r, Instant::now());
                            rig.run(fx);
                        }
                    }
                    Reply::Personas(list) => {
                        let items = parse_personas(&serde_json::Value::Array(list));
                        rig.send_ss(SsMsg::Personas(items));
                    }
                    Reply::Failed(why) => {
                        gtk::glib::g_warning!("agent-shell", "tools command failed: {why}")
                    }
                }
                rig.refresh();
            }
        });
    }

    /// Controller views (for UI scenarios).
    pub(crate) fn tools_view(&self) -> controllers::controllers::tools::ToolsView {
        self.tabs.borrow().view()
    }

    pub(crate) fn browser_view(&self) -> controllers::browser_view::BrowserView {
        self.ctls.borrow().browser.view()
    }

    pub(crate) fn terminal_view(&self) -> controllers::terminal_view::TerminalView {
        self.ctls.borrow().terminal.view()
    }

    pub(crate) fn settings_view(&self) -> controllers::ss_view::SsView {
        self.ctls.borrow().settings.view()
    }
}
