//! Browser start page: dark, logo, title, centred search, quick links and "Add".
//! Drawn with GTK widgets, so it is visible without a network. All text is without markup.

use std::cell::RefCell;
use std::rc::Rc;

use controllers::browser_view::BrowserMsg;
use controllers::i18n::tr;
use controllers::tool_types::Link;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

pub struct StartPage {
    pub root: gtk::Box,
    search: gtk::Entry,
    quick: gtk::FlowBox,
    title: gtk::Entry,
    url: gtk::Entry,
    /// "Add" a quick link.
    pub add: gtk::Button,
    shown: RefCell<Vec<Link>>,
}

impl StartPage {
    pub fn new(send: Rc<dyn Fn(BrowserMsg)>) -> Rc<StartPage> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("browser-start");
        root.set_valign(gtk::Align::Center);
        root.set_halign(gtk::Align::Center);
        root.set_margin_start(16);
        root.set_margin_end(16);
        a11y::set_name(&root, &tr("browser-start-name"));
        let logo = gtk::Image::from_icon_name("web-browser-symbolic");
        logo.set_pixel_size(48);
        let name = gtk::Label::new(Some(&tr("browser-start-title")));
        name.set_use_markup(false);
        name.add_css_class("title-1");
        let search = gtk::Entry::new();
        search.set_placeholder_text(Some(&tr("browser-search-placeholder")));
        search.set_width_chars(30);
        a11y::set_name(&search, &tr("browser-search-name"));
        let quick = gtk::FlowBox::new();
        quick.set_selection_mode(gtk::SelectionMode::None);
        quick.set_max_children_per_line(5);
        quick.set_homogeneous(false);
        a11y::set_name(&quick, &tr("browser-quick-name"));
        let add = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = gtk::Entry::new();
        title.set_placeholder_text(Some(&tr("browser-quick-title-ph")));
        a11y::set_name(&title, &tr("browser-quick-title-ph"));
        let url = gtk::Entry::new();
        url.set_placeholder_text(Some(&tr("browser-quick-url-ph")));
        a11y::set_name(&url, &tr("browser-quick-url-ph"));
        let button = gtk::Button::with_label(&tr("browser-add-quick"));
        button.add_css_class("shell-btn");
        a11y::set_name(&button, &tr("browser-add-quick"));
        add.append(&title);
        add.append(&url);
        add.append(&button);
        for w in [
            logo.upcast_ref::<gtk::Widget>(),
            name.upcast_ref(),
            search.upcast_ref(),
            quick.upcast_ref(),
            add.upcast_ref(),
        ] {
            root.append(w);
        }
        let page = Rc::new(StartPage {
            root,
            search,
            quick,
            title,
            url,
            add: button.clone(),
            shown: RefCell::new(Vec::new()),
        });
        let s = send.clone();
        page.search.connect_activate(move |e| {
            s(BrowserMsg::Submit(e.text().to_string()));
            e.set_text("");
        });
        let weak = Rc::downgrade(&page);
        button.connect_clicked(move |_| {
            let Some(p) = weak.upgrade() else { return };
            send(BrowserMsg::AddQuick {
                title: p.title.text().to_string(),
                url: p.url.text().to_string(),
            });
            p.title.set_text("");
            p.url.set_text("");
        });
        page
    }

    /// Rebuilds the quick link row only if the list changed.
    pub fn set_quick(&self, links: &[Link], send: &Rc<dyn Fn(BrowserMsg)>) {
        if *self.shown.borrow() == links {
            return;
        }
        *self.shown.borrow_mut() = links.to_vec();
        while let Some(child) = self.quick.first_child() {
            self.quick.remove(&child);
        }
        for link in links {
            let button = gtk::Button::with_label(&link.title);
            button.add_css_class("shell-btn");
            button.set_tooltip_text(Some(&link.url));
            a11y::set_name(&button, &link.title);
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_use_markup(false);
            }
            let (send, url) = (send.clone(), link.url.clone());
            button.connect_clicked(move |_| send(BrowserMsg::Submit(url.clone())));
            self.quick.append(&button);
        }
    }

    /// Quick link titles on screen (for UI scenarios).
    pub fn quick_titles(&self) -> Vec<String> {
        self.shown
            .borrow()
            .iter()
            .map(|l| l.title.clone())
            .collect()
    }

    pub fn fill_add(&self, title: &str, url: &str) {
        self.title.set_text(title);
        self.url.set_text(url);
    }
}
