//! Access to shell parts for UI scenarios and tests (widgets, views, bus): thin wrappers over `Inner`.

use std::rc::Rc;

use controllers::{Column, Controller, EventBus, Theme};
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::notif_toast::ToastView;
use crate::shell::{ColumnId, Shell};

impl Shell {
    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.inner.window
    }

    pub fn column(&self, id: ColumnId) -> gtk::Widget {
        self.inner.column(id).root.clone().upcast()
    }

    pub fn bar(&self) -> gtk::Widget {
        self.inner.bar.root.clone().upcast()
    }

    /// Column collapse button (bottom of the column).
    pub fn collapse_button(&self, column: Column) -> gtk::Button {
        let id = match column {
            Column::Notifications => ColumnId::Notifications,
            Column::Sessions => ColumnId::Sessions,
            Column::Tools => ColumnId::Tools,
        };
        self.inner.column(id).collapse.clone().unwrap_or_default()
    }

    pub fn theme_button(&self) -> gtk::Button {
        self.inner.bar.theme_button.clone()
    }

    pub fn selected_theme(&self) -> Theme {
        self.inner.theme.borrow().view().selected
    }

    /// Internal event bus (tests subscribe to `OpenSettings` and others).
    pub fn bus(&self) -> EventBus {
        self.inner.bus.clone()
    }

    pub fn menu_button(&self) -> gtk::Button {
        self.inner.bar.widgets.menu_button.clone()
    }

    pub fn menu_item(&self, item: controllers::MenuItem) -> Option<gtk::Button> {
        self.inner.bar_rig.menu_item_button(item)
    }

    pub fn quota_button(&self, account_id: i64) -> Option<gtk::Button> {
        self.inner.bar.widgets.quota_button(account_id)
    }

    /// Dialog layer over the window (backdrop and card).
    pub fn dialog_layer(&self) -> gtk::Box {
        self.inner.bar_rig.host.layer.clone()
    }

    /// The Agent OS menu layer is visible.
    pub fn menu_open(&self) -> bool {
        self.inner.bar.widgets.menu_layer.is_visible()
    }

    pub fn menu_layer(&self) -> gtk::Box {
        self.inner.bar.widgets.menu_layer.clone()
    }

    pub fn dialog_open(&self) -> bool {
        self.inner.bar_rig.host.is_open()
    }

    pub fn dialog_texts(&self) -> Vec<String> {
        self.inner.bar_rig.host.texts()
    }

    pub fn dialog_button(&self, label: &str) -> Option<gtk::Button> {
        self.inner.bar_rig.host.find_button(label)
    }

    /// Sessions column widgets (for UI scenarios).
    pub fn tree(&self) -> Rc<crate::tree_widgets::TreeWidgets> {
        self.inner.tree_rig.widgets.clone()
    }

    /// Session tree view (for UI scenarios).
    pub fn tree_view(&self) -> controllers::tree_view::TreeView {
        self.inner.tree_rig.view()
    }

    /// "Chat" column widgets (for UI scenarios).
    pub fn chat_widgets(&self) -> Rc<crate::chat_widgets::ChatWidgets> {
        self.inner.chat_rig.widgets.clone()
    }

    /// "Notifications" column widgets (for UI scenarios).
    pub fn notif_widgets(&self) -> Rc<crate::notif_widgets::NotifWidgets> {
        self.inner.notif_rig.widgets.clone()
    }

    /// Notification feed view (for UI scenarios).
    pub fn notif_view(&self) -> controllers::notif_view::NotifView {
        self.inner.notif_rig.view()
    }

    /// Tools column widgets (for UI scenarios).
    pub fn tools_widgets(&self) -> Rc<crate::tools_widgets::ToolsWidgets> {
        self.inner.tools_rig.widgets.clone()
    }

    pub fn browser_pane(&self) -> Rc<crate::browser_pane::BrowserPane> {
        self.inner.tools_rig.browser.clone()
    }

    pub fn terminal_pane(&self) -> Rc<crate::terminal_pane::TerminalPane> {
        self.inner.tools_rig.terminal.clone()
    }

    /// Settings dialog (for UI scenarios).
    pub fn settings_dialog(&self) -> Rc<crate::settings_widgets::SettingsDialog> {
        self.inner.settings_rig.dialog.clone()
    }

    /// Settings dialog view (for UI scenarios).
    pub fn settings_dialog_view(&self) -> controllers::settings_view::SettingsView {
        self.inner.settings_rig.view()
    }

    /// Message to the Settings dialog (for UI scenarios and tests).
    pub fn settings_send(&self, msg: controllers::settings_view::SettingsMsg) {
        self.inner.settings_rig.send(msg);
    }

    pub fn settings_pane(&self) -> Rc<crate::ss_pane::SettingsPane> {
        self.inner.tools_rig.settings.clone()
    }

    /// Tool controller views (for UI scenarios).
    pub fn tools_view(&self) -> controllers::controllers::tools::ToolsView {
        self.inner.tools_rig.tools_view()
    }

    pub fn browser_view(&self) -> controllers::browser_view::BrowserView {
        self.inner.tools_rig.browser_view()
    }

    pub fn terminal_view(&self) -> controllers::terminal_view::TerminalView {
        self.inner.tools_rig.terminal_view()
    }

    pub fn settings_view(&self) -> controllers::ss_view::SsView {
        self.inner.tools_rig.settings_view()
    }

    /// Toast (for UI scenarios).
    pub fn toast(&self) -> Rc<ToastView> {
        self.inner.toast.clone()
    }

    /// Chat view (for UI scenarios).
    pub fn chat_view(&self) -> controllers::chat_view::ChatView {
        self.inner.chat_rig.view()
    }
}
