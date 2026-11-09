//! Session tree row: widgets, update from `TreeRow`, actions on hover and focus, keys.
//! User text is shown as plain text (no Pango markup), a persona colour is validated as `#rrggbb`.

use std::cell::Cell;
use std::rc::Rc;

use controllers::controllers::session_tree::TreeMsg;
use controllers::tree_types::NodeId;
use controllers::tree_view::{RowKind, TreeRow};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::theme_tokens::Rgb;
use crate::tree_row_input::{actions_for, connect_row};

pub type Sender = Rc<dyn Fn(TreeMsg)>;

/// Persona icon size, pixels.
const DOT: i32 = 10;

pub(crate) struct RowWidgets {
    pub(crate) node: NodeId,
    pub(crate) root: gtk::Box,
    arrow: gtk::Image,
    dot: gtk::DrawingArea,
    color: Rc<Cell<Option<Rgb>>>,
    name: gtk::Label,
    badge: gtk::Label,
}

pub(crate) fn plain(css: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_selectable(false);
    label.add_css_class(css);
    label
}

pub(crate) fn update_row(w: &RowWidgets, r: &TreeRow) {
    w.name.set_text(&r.name);
    w.name.set_tooltip_text(None);
    w.root.set_margin_start(r.indent as i32);
    a11y::set_name(&w.root, &r.a11y_name);
    w.root.update_state(&[
        gtk::accessible::State::Selected(Some(r.selected)),
        gtk::accessible::State::Expanded(r.expanded),
    ]);
    if let Some(open) = r.expanded {
        w.arrow.set_icon_name(Some(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    }
    if r.selected {
        w.root.add_css_class("selected");
    } else {
        w.root.remove_css_class("selected");
    }
    match &r.badge {
        Some(text) => {
            w.badge.set_text(text);
            w.badge.set_visible(true);
        }
        None => w.badge.set_visible(false),
    }
    let color = r.persona_color.as_deref().and_then(Rgb::parse);
    w.color.set(color);
    w.dot.set_visible(color.is_some());
    w.dot.queue_draw();
}

pub(crate) fn build_row(send: &Sender, quiet: &Rc<Cell<bool>>, r: &TreeRow) -> RowWidgets {
    let role = gtk::AccessibleRole::TreeItem;
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .accessible_role(role)
        .spacing(6)
        .build();
    root.add_css_class("tree-row");
    let arrow = gtk::Image::new();
    arrow.set_pixel_size(12);
    arrow.set_opacity(if r.kind == RowKind::Folder { 1.0 } else { 0.0 });
    let icon = gtk::Image::from_icon_name(match r.kind {
        RowKind::Folder => "folder-symbolic",
        RowKind::Session => "chat-message-new-symbolic",
        RowKind::Thread => "mail-reply-all-symbolic",
    });
    icon.set_pixel_size(14);
    let color: Rc<Cell<Option<Rgb>>> = Rc::new(Cell::new(None));
    let dot = gtk::DrawingArea::new();
    dot.set_content_width(DOT);
    dot.set_content_height(DOT);
    dot.set_valign(gtk::Align::Center);
    let draw_color = color.clone();
    dot.set_draw_func(move |_, cr, w, h| {
        let Some(Rgb(red, green, blue)) = draw_color.get() else {
            return;
        };
        cr.set_source_rgb(
            f64::from(red) / 255.0,
            f64::from(green) / 255.0,
            f64::from(blue) / 255.0,
        );
        cr.arc(
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            f64::from(w.min(h)) / 2.0,
            0.0,
            std::f64::consts::TAU,
        );
        if let Err(e) = cr.fill() {
            gtk::glib::g_warning!("agent-shell", "persona dot not drawn: {e}");
        }
    });
    let name = plain("tree-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let badge = plain("tree-badge");
    badge.set_visible(false);
    let actions = actions_for(send, r);
    for w in [
        arrow.upcast_ref::<gtk::Widget>(),
        icon.upcast_ref(),
        dot.upcast_ref(),
        name.upcast_ref(),
        badge.upcast_ref(),
        actions.upcast_ref(),
    ] {
        root.append(w);
    }
    connect_row(send, quiet, &root, &actions, r.node);
    RowWidgets {
        node: r.node,
        root,
        arrow,
        dot,
        color,
        name,
        badge,
    }
}
