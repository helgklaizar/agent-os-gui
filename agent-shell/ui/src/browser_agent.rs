//! Agent commands to the browser (api.md section 5.2.1): the page loads in a separate profile (the user's cookies are not
//! available to it), the page text is returned to the core as a result through `BrowserController`. One request at a time.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use controllers::browser_view::{AgentLoad, BrowserMsg, agent_profile_dir};
use controllers::tool_types::ToolResult;
use gtk4 as gtk;
use gtk4::prelude::*;
use webkit6::prelude::*;

/// How long we wait for the page load and text, then the result is `timeout`.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct AgentRunner {
    /// Hidden container: WebKit loads the page while it is in the widget tree.
    pub host: gtk::Box,
    data_dir: PathBuf,
    send: Rc<dyn Fn(BrowserMsg)>,
    current: RefCell<Option<String>>,
    view: RefCell<Option<webkit6::WebView>>,
}

impl AgentRunner {
    pub fn new(data_dir: PathBuf, send: Rc<dyn Fn(BrowserMsg)>) -> AgentRunner {
        let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        host.set_size_request(1, 1);
        host.set_overflow(gtk::Overflow::Hidden);
        host.set_opacity(0.0);
        host.set_can_target(false);
        host.set_focusable(false);
        host.set_accessible_role(gtk::AccessibleRole::Presentation);
        AgentRunner {
            host,
            data_dir,
            send,
            current: RefCell::new(None),
            view: RefCell::new(None),
        }
    }

    /// A new command starts a load; the same command again (a repeated `apply` call) does nothing.
    pub fn apply(&self, load: Option<&AgentLoad>) {
        let Some(load) = load else { return };
        if self.current.borrow().as_deref() == Some(&load.request_id) {
            return;
        }
        *self.current.borrow_mut() = Some(load.request_id.clone());
        if let Some(old) = self.view.borrow_mut().take() {
            self.host.remove(&old);
        }
        let dir = agent_profile_dir(&self.data_dir, load.session_id);
        let (data, cache) = (dir.join("data"), dir.join("cache"));
        for d in [&data, &cache] {
            if let Err(e) = std::fs::create_dir_all(d) {
                gtk::glib::g_warning!("agent-shell", "agent profile dir not created: {e}");
            }
        }
        let net = webkit6::NetworkSession::new(
            Some(&data.to_string_lossy()),
            Some(&cache.to_string_lossy()),
        );
        let wv = webkit6::WebView::builder().network_session(&net).build();
        self.host.append(&wv);
        *self.view.borrow_mut() = Some(wv.clone());
        let done = Rc::new(Cell::new(false));
        let started = Instant::now();
        let (send, session_id, rid, url) = (
            self.send.clone(),
            load.session_id,
            load.request_id.clone(),
            load.url.clone(),
        );
        let finish = {
            let (send, rid, done) = (send.clone(), rid.clone(), done.clone());
            move |result: ToolResult| {
                if !done.replace(true) {
                    send(BrowserMsg::AgentDone { session_id, result });
                }
                let _ = &rid;
            }
        };
        let (f, r, u, d) = (finish.clone(), rid.clone(), url.clone(), done.clone());
        wv.connect_load_changed(move |wv, ev| {
            if ev != webkit6::LoadEvent::Finished || d.get() {
                return;
            }
            let (f, r, u) = (f.clone(), r.clone(), u.clone());
            wv.evaluate_javascript(
                "document.documentElement.outerHTML",
                None,
                None,
                None::<&gtk::gio::Cancellable>,
                move |res| {
                    let ms = started.elapsed().as_millis() as u64;
                    f(match res.ok().map(|v| v.to_str().to_string()) {
                        Some(text) => ToolResult::page(&r, &u, 200, &text, ms),
                        None => ToolResult::failed(&r, "error", "page_not_read"),
                    });
                },
            );
        });
        let (f, r) = (finish.clone(), rid.clone());
        wv.connect_load_failed(move |_, _, _, _| {
            f(ToolResult::failed(&r, "error", "load_failed"));
            false
        });
        let (f, r, d) = (finish, rid, done);
        gtk::glib::timeout_add_local_once(TIMEOUT, move || {
            if !d.get() {
                f(ToolResult::failed(&r, "timeout", "timeout"));
            }
        });
        wv.load_uri(&url);
    }
}
