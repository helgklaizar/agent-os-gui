//! Skeleton widgets: the bar and four columns. No layout logic here: widths and collapsing are decided by `controllers`.

use std::rc::Rc;

use controllers::i18n::{tr, tr_args};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::bar_view::BarWidgets;
use crate::shell::ColumnId;

/// Bar height (spec section 5).
pub const BAR_HEIGHT: i32 = 24;
/// Column header height.
const HEADER_HEIGHT: i32 = 44;

pub fn column_title(id: ColumnId) -> String {
    match id {
        ColumnId::Notifications => tr("column-notifications"),
        ColumnId::Sessions => tr("column-sessions"),
        ColumnId::Chat => tr("column-chat"),
        ColumnId::Tools => tr("column-tools"),
    }
}

fn column_icon(id: ColumnId) -> &'static str {
    match id {
        ColumnId::Notifications => "preferences-system-notifications-symbolic",
        ColumnId::Sessions => "view-list-symbolic",
        ColumnId::Chat => "chat-message-new-symbolic",
        ColumnId::Tools => "view-dual-symbolic",
    }
}

/// Widgets of one column.
pub struct Column {
    pub id: ColumnId,
    pub root: gtk::Box,
    title: gtk::Label,
    pub body: gtk::Box,
    /// Unread badge on the column icon (visible when there is something to show).
    pub badge: gtk::Label,
    pub collapse: Option<gtk::Button>,
}

impl Column {
    pub fn new(id: ColumnId) -> Column {
        let name = column_title(id);
        let root = a11y::region(&name);
        root.add_css_class("column");
        root.add_css_class(if id == ColumnId::Tools {
            "column-end"
        } else {
            "column-start"
        });
        root.set_hexpand(id == ColumnId::Chat);
        root.set_overflow(gtk::Overflow::Hidden);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.add_css_class("column-header");
        header.set_size_request(-1, HEADER_HEIGHT);
        let icon = gtk::Image::from_icon_name(column_icon(id));
        let badge = gtk::Label::new(None);
        badge.set_use_markup(false);
        badge.add_css_class("tree-badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_visible(false);
        let icon_overlay = gtk::Overlay::new();
        icon_overlay.set_child(Some(&icon));
        icon_overlay.add_overlay(&badge);
        icon_overlay.set_margin_start(12);
        icon_overlay.set_margin_end(4);
        let title = gtk::Label::new(Some(&name));
        title.add_css_class("column-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.append(&icon_overlay);
        header.append(&title);
        root.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_vexpand(true);
        root.append(&body);

        let collapse = (id != ColumnId::Chat).then(|| {
            let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            footer.add_css_class("column-footer");
            footer.set_halign(gtk::Align::Fill);
            let button = a11y::icon_button("pan-start-symbolic", "");
            button.add_css_class("shell-btn");
            button.set_margin_top(6);
            button.set_margin_bottom(6);
            button.set_margin_start(6);
            button.set_margin_end(6);
            button.set_hexpand(true);
            button.set_halign(gtk::Align::End);
            footer.append(&button);
            root.append(&footer);
            button
        });
        let column = Column {
            id,
            root,
            title,
            body,
            badge,
            collapse,
        };
        column.set_collapsed(false);
        column
    }

    /// Collapsed column: only the icon and a button; button and tooltip names change to "Expand...".
    pub fn set_collapsed(&self, collapsed: bool) {
        self.title.set_visible(!collapsed);
        // the bodies of the sessions, notifications and tools columns show the collapsed strip themselves
        self.body.set_visible(
            !collapsed
                || matches!(
                    self.id,
                    ColumnId::Sessions | ColumnId::Notifications | ColumnId::Tools
                ),
        );
        if collapsed {
            self.root.add_css_class("collapsed");
        } else {
            self.root.remove_css_class("collapsed");
        }
        let Some(button) = &self.collapse else { return };
        let pointing_end = (self.id == ColumnId::Tools) != collapsed;
        button.set_icon_name(if pointing_end {
            "pan-end-symbolic"
        } else {
            "pan-start-symbolic"
        });
        let column = column_title(self.id);
        let name = if collapsed {
            tr_args("column-expand", &[("column", &column)])
        } else {
            tr_args("column-collapse", &[("column", &column)])
        };
        a11y::set_name(button, &name);
        button.set_tooltip_text(Some(&name));
        button.set_halign(if collapsed {
            gtk::Align::Center
        } else {
            gtk::Align::End
        });
    }
}

/// Status bar: Agent OS menu, quotas, metrics, weather, date, clock and theme switch.
pub struct Bar {
    pub root: gtk::Box,
    pub theme_button: gtk::Button,
    pub widgets: Rc<BarWidgets>,
}

impl Bar {
    pub fn new() -> Bar {
        // Banner in GTK 4.14 does not accept a name (it stays empty in AT-SPI), so a region is used, as with the columns
        let root = a11y::landmark(
            gtk::AccessibleRole::Region,
            gtk::Orientation::Horizontal,
            &tr("bar-name"),
        );
        root.add_css_class("bar");
        root.set_size_request(-1, BAR_HEIGHT);
        root.set_vexpand(false);
        root.set_overflow(gtk::Overflow::Hidden);
        let theme_button = gtk::Button::new();
        theme_button.add_css_class("bar-btn");
        theme_button.set_margin_end(6);
        let widgets = Rc::new(BarWidgets::build(&root, &theme_button));
        Bar {
            root,
            theme_button,
            widgets,
        }
    }
}

impl Default for Bar {
    fn default() -> Self {
        Self::new()
    }
}
