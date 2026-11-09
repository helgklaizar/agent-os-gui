//! Tools column skeleton: icon tabs on top (a vertical strip when collapsed) and a stack of pages.
//! `ToolsController` decides the logic (which tab is selected, whether the column is visible); this file has widgets only.

use std::cell::Cell;
use std::rc::Rc;

use controllers::controllers::tools::{TOOL_TABS, ToolsView};
use controllers::{Tab, i18n::tr};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

fn icon(tab: Tab) -> &'static str {
    match tab {
        Tab::Browser => "web-browser-symbolic",
        Tab::Terminal => "utilities-terminal-symbolic",
        _ => "emblem-system-symbolic",
    }
}

fn page_name(tab: Tab) -> &'static str {
    match tab {
        Tab::Browser => "browser",
        Tab::Terminal => "terminal",
        _ => "settings",
    }
}

pub struct ToolsWidgets {
    pub root: gtk::Box,
    pub bar: gtk::Box,
    pub stack: gtk::Stack,
    pub buttons: Vec<(Tab, gtk::ToggleButton)>,
    /// Buttons must not react to `toggled` while the view itself switches them.
    syncing: Cell<bool>,
    on_select: Rc<dyn Fn(Tab)>,
}

impl ToolsWidgets {
    pub fn new(on_select: Rc<dyn Fn(Tab)>) -> Rc<ToolsWidgets> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tools");
        root.set_vexpand(true);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.add_css_class("tools-tabs");
        bar.set_margin_start(6);
        bar.set_margin_end(6);
        bar.set_margin_top(4);
        bar.set_margin_bottom(4);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);
        let mut buttons = Vec::new();
        let mut group: Option<gtk::ToggleButton> = None;
        for tab in TOOL_TABS {
            let name = match tab {
                Tab::Browser => tr("tools-tab-browser"),
                Tab::Terminal => tr("tools-tab-terminal"),
                _ => tr("tools-tab-settings"),
            };
            let button = gtk::ToggleButton::new();
            button.set_icon_name(icon(tab));
            button.add_css_class("shell-btn");
            button.set_tooltip_text(Some(&name));
            a11y::set_name(&button, &name);
            if let Some(g) = &group {
                button.set_group(Some(g));
            } else {
                group = Some(button.clone());
            }
            bar.append(&button);
            buttons.push((tab, button));
        }
        root.append(&bar);
        root.append(&stack);
        let widgets = Rc::new(ToolsWidgets {
            root,
            bar,
            stack,
            buttons,
            syncing: Cell::new(false),
            on_select,
        });
        for (tab, button) in &widgets.buttons {
            let weak = Rc::downgrade(&widgets);
            let tab = *tab;
            button.connect_toggled(move |b| {
                let Some(w) = weak.upgrade() else { return };
                if b.is_active() && !w.syncing.get() {
                    (w.on_select)(tab);
                }
            });
        }
        widgets
    }

    pub fn add_page(&self, tab: Tab, page: &impl IsA<gtk::Widget>) {
        self.stack.add_named(page, Some(page_name(tab)));
    }

    /// Applies the view: selected tab, collapsed strip (vertical), column visibility.
    pub fn apply(&self, view: &ToolsView) {
        self.syncing.set(true);
        for (tab, button) in &self.buttons {
            button.set_active(*tab == view.active);
        }
        self.syncing.set(false);
        self.stack.set_visible_child_name(page_name(view.active));
        self.stack.set_visible(!view.collapsed);
        self.bar.set_orientation(if view.collapsed {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
        self.bar.set_halign(if view.collapsed {
            gtk::Align::Center
        } else {
            gtk::Align::Start
        });
        self.root.set_visible(view.visible);
    }

    pub fn button(&self, tab: Tab) -> Option<gtk::ToggleButton> {
        self.buttons
            .iter()
            .find(|(t, _)| *t == tab)
            .map(|(_, b)| b.clone())
    }
}
