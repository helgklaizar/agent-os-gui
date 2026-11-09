//! "Browser" tab: address bar, back / forward / reload / home, bookmarks, start
//! page and a WebKit page per session with its own profile. External pages live only in WebKit.
//! `BrowserController` decides what to load and when; this file has widgets and WebKit signals.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use controllers::browser_addr::is_loadable;
use controllers::browser_view::{BrowserMsg, BrowserView, profile_dir};
use controllers::i18n::tr;
use gtk4 as gtk;
use gtk4::prelude::*;
use webkit6::prelude::*;

use crate::a11y;
use crate::browser_agent::AgentRunner;
use crate::browser_start::StartPage;

pub struct BrowserPane {
    pub root: gtk::Box,
    pub back: gtk::Button,
    pub forward: gtk::Button,
    pub reload: gtk::Button,
    pub home: gtk::Button,
    pub mark: gtk::Button,
    pub marks: gtk::MenuButton,
    pub address: gtk::Entry,
    pub start: Rc<StartPage>,
    pages: gtk::Stack,
    marks_box: gtk::Box,
    webviews: RefCell<HashMap<i64, webkit6::WebView>>,
    agent: AgentRunner,
    data_dir: PathBuf,
    applied_seq: Cell<u64>,
    last_marks: RefCell<Vec<controllers::tool_types::Link>>,
    send: Rc<dyn Fn(BrowserMsg)>,
}

fn nav_button(icon: &str, name: &str) -> gtk::Button {
    let b = a11y::icon_button(icon, name);
    b.add_css_class("shell-btn");
    b
}

impl BrowserPane {
    pub fn new(data_dir: PathBuf, send: Rc<dyn Fn(BrowserMsg)>) -> Rc<BrowserPane> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.set_margin_start(6);
        bar.set_margin_end(6);
        bar.set_margin_bottom(4);
        let back = nav_button("go-previous-symbolic", &tr("browser-back"));
        let forward = nav_button("go-next-symbolic", &tr("browser-forward"));
        let reload = nav_button("view-refresh-symbolic", &tr("browser-reload"));
        let home = nav_button("go-home-symbolic", &tr("browser-home"));
        let mark = nav_button("non-starred-symbolic", &tr("browser-bookmark-add"));
        let address = gtk::Entry::new();
        address.set_hexpand(true);
        address.set_placeholder_text(Some(&tr("browser-address-name")));
        a11y::set_name(&address, &tr("browser-address-name"));
        let marks = gtk::MenuButton::new();
        marks.set_icon_name("user-bookmarks-symbolic");
        marks.add_css_class("shell-btn");
        marks.set_tooltip_text(Some(&tr("browser-bookmarks")));
        a11y::set_name(&marks, &tr("browser-bookmarks"));
        let marks_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&marks_box));
        marks.set_popover(Some(&popover));
        for w in [
            back.upcast_ref::<gtk::Widget>(),
            forward.upcast_ref(),
            reload.upcast_ref(),
            home.upcast_ref(),
            address.upcast_ref(),
            mark.upcast_ref(),
            marks.upcast_ref(),
        ] {
            bar.append(w);
        }
        let start = StartPage::new(send.clone());
        let pages = gtk::Stack::new();
        pages.set_vexpand(true);
        pages.add_named(&start.root, Some("start"));
        let agent = AgentRunner::new(data_dir.clone(), send.clone());
        root.append(&bar);
        root.append(&pages);
        root.append(&agent.host);
        let pane = Rc::new(BrowserPane {
            root,
            back,
            forward,
            reload,
            home,
            mark,
            marks,
            address,
            start,
            pages,
            marks_box,
            webviews: RefCell::new(HashMap::new()),
            agent,
            data_dir,
            applied_seq: Cell::new(0),
            last_marks: RefCell::new(Vec::new()),
            send,
        });
        pane.connect();
        pane
    }

    fn connect(self: &Rc<Self>) {
        let s = self.send.clone();
        self.back.connect_clicked(move |_| s(BrowserMsg::Back));
        let s = self.send.clone();
        self.forward
            .connect_clicked(move |_| s(BrowserMsg::Forward));
        let s = self.send.clone();
        self.reload.connect_clicked(move |_| s(BrowserMsg::Reload));
        let s = self.send.clone();
        self.home.connect_clicked(move |_| s(BrowserMsg::Home));
        let s = self.send.clone();
        self.address
            .connect_activate(move |e| s(BrowserMsg::Submit(e.text().to_string())));
    }

    /// A session page with its own profile: cookies, data and history are not mixed between sessions.
    fn webview(self: &Rc<Self>, session_id: i64) -> webkit6::WebView {
        if let Some(wv) = self.webviews.borrow().get(&session_id) {
            return wv.clone();
        }
        let dir = profile_dir(&self.data_dir, session_id);
        let (data, cache) = (dir.join("data"), dir.join("cache"));
        for d in [&data, &cache] {
            if let Err(e) = std::fs::create_dir_all(d) {
                gtk::glib::g_warning!("agent-shell", "browser profile dir not created: {e}");
            }
        }
        let net = webkit6::NetworkSession::new(
            Some(&data.to_string_lossy()),
            Some(&cache.to_string_lossy()),
        );
        let wv = webkit6::WebView::builder().network_session(&net).build();
        wv.set_vexpand(true);
        let send = self.send.clone();
        let report = move |wv: &webkit6::WebView| {
            if let Some(uri) = wv.uri() {
                send(BrowserMsg::Loaded {
                    session_id,
                    url: uri.to_string(),
                    title: wv.title().map(|t| t.to_string()).unwrap_or_default(),
                });
            }
        };
        let r = report.clone();
        wv.connect_load_changed(move |wv, ev| {
            if ev == webkit6::LoadEvent::Committed {
                r(wv);
            }
        });
        wv.connect_title_notify(move |wv| report(wv));
        // schemes other than http and https are not opened in the page
        wv.connect_decide_policy(|_, decision, kind| {
            if kind != webkit6::PolicyDecisionType::NavigationAction {
                return false;
            }
            let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
                return false;
            };
            let uri = nav
                .navigation_action()
                .and_then(|a| a.request())
                .and_then(|r| r.uri());
            if uri.is_some_and(|u| is_loadable(&u)) {
                false
            } else {
                decision.ignore();
                true
            }
        });
        self.pages.add_named(&wv, Some(&format!("s{session_id}")));
        self.webviews.borrow_mut().insert(session_id, wv.clone());
        wv
    }

    fn rebuild_marks(&self, view: &BrowserView) {
        if *self.last_marks.borrow() == view.bookmarks {
            return;
        }
        *self.last_marks.borrow_mut() = view.bookmarks.clone();
        while let Some(c) = self.marks_box.first_child() {
            self.marks_box.remove(&c);
        }
        for (i, link) in view.bookmarks.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let open = gtk::Button::with_label(&link.title);
            open.set_hexpand(true);
            open.set_tooltip_text(Some(&link.url));
            if let Some(l) = open.child().and_downcast::<gtk::Label>() {
                l.set_use_markup(false);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(32);
            }
            let (s, url) = (self.send.clone(), link.url.clone());
            open.connect_clicked(move |_| s(BrowserMsg::Submit(url.clone())));
            let del = a11y::icon_button("window-close-symbolic", &tr("browser-bookmark-remove"));
            let s = self.send.clone();
            del.connect_clicked(move |_| s(BrowserMsg::RemoveMark(i)));
            row.append(&open);
            row.append(&del);
            self.marks_box.append(&row);
        }
    }

    /// Applies the view: page or start, address, buttons, bookmarks, new load and agent command.
    pub fn apply(self: &Rc<Self>, view: &BrowserView) {
        self.back.set_sensitive(view.can_back);
        self.forward.set_sensitive(view.can_forward);
        self.reload.set_sensitive(!view.start_page);
        self.mark.set_sensitive(!view.start_page);
        self.mark.set_icon_name(if view.bookmarked {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        let mark_name = if view.bookmarked {
            tr("browser-bookmark-remove")
        } else {
            tr("browser-bookmark-add")
        };
        a11y::set_name(&self.mark, &mark_name);
        if self.address.text() != view.address {
            self.address.set_text(&view.address);
        }
        self.start.set_quick(&view.quick, &self.send);
        self.rebuild_marks(view);
        if let Some(load) = &view.load
            && load.seq > self.applied_seq.get()
        {
            self.applied_seq.set(load.seq);
            let wv = self.webview(load.session_id);
            if is_loadable(&load.url) {
                wv.load_uri(&load.url);
            }
        }
        match view.session {
            Some(id) if !view.start_page && self.webviews.borrow().contains_key(&id) => {
                self.pages.set_visible_child_name(&format!("s{id}"));
            }
            _ => self.pages.set_visible_child_name("start"),
        }
        self.agent.apply(view.agent.as_ref());
    }

    /// Whether the address is shown now (for UI scenarios).
    pub fn shows_page_of(&self, session_id: i64) -> bool {
        self.pages.visible_child_name().as_deref() == Some(&format!("s{session_id}"))
    }

    pub fn has_webview(&self, session_id: i64) -> bool {
        self.webviews.borrow().contains_key(&session_id)
    }
}
