//! Tree row actions and input: buttons on hover and focus, click, arrows, Enter.

use std::cell::Cell;
use std::rc::Rc;

use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::i18n::tr;
use controllers::tree_types::NodeId;
use controllers::tree_view::{NavKey, RowKind, TreeRow};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;
use crate::tree_row::Sender;

fn action(icon: &str, name: &str, send: &Sender, msg: TreeMsg) -> gtk::Button {
    let button = a11y::icon_button(icon, name);
    button.add_css_class("flat");
    button.add_css_class("tree-action");
    let send = send.clone();
    button.connect_clicked(move |_| send(msg.clone()));
    button
}

fn nav_key(key: gtk::gdk::Key) -> Option<NavKey> {
    use gtk::gdk::Key;
    Some(match key {
        Key::Up => NavKey::Up,
        Key::Down => NavKey::Down,
        Key::Left => NavKey::Left,
        Key::Right => NavKey::Right,
        Key::Home => NavKey::Home,
        Key::End => NavKey::End,
        Key::Return | Key::KP_Enter => NavKey::Enter,
        _ => return None,
    })
}

pub(crate) fn actions_for(send: &Sender, r: &TreeRow) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.set_visible(false);
    let s = send;
    let req = |q| TreeMsg::Request(q);
    let items: Vec<gtk::Button> = match r.node {
        NodeId::Folder(id) => vec![
            action(
                "document-new-symbolic",
                &tr("tree-action-new-session"),
                s,
                req(TreeRequest::NewSessionInFolder { folder: id }),
            ),
            action(
                "folder-new-symbolic",
                &tr("tree-action-subfolder"),
                s,
                req(TreeRequest::NewSubfolder { parent: id }),
            ),
            action(
                "document-edit-symbolic",
                &tr("tree-action-rename"),
                s,
                req(TreeRequest::Rename { node: r.node }),
            ),
            action(
                "edit-delete-symbolic",
                &tr("tree-action-delete"),
                s,
                req(TreeRequest::Delete { node: r.node }),
            ),
        ],
        NodeId::Session(id) if r.kind == RowKind::Session => vec![
            action(
                "list-add-symbolic",
                &tr("tree-action-new-thread"),
                s,
                req(TreeRequest::NewThread { parent: id }),
            ),
            action(
                "folder-open-symbolic",
                &tr("tree-action-move"),
                s,
                req(TreeRequest::Move { session: id }),
            ),
            action(
                "document-edit-symbolic",
                &tr("tree-action-rename"),
                s,
                req(TreeRequest::Rename { node: r.node }),
            ),
            action(
                "edit-delete-symbolic",
                &tr("tree-action-delete"),
                s,
                req(TreeRequest::Delete { node: r.node }),
            ),
        ],
        NodeId::Session(_) => vec![
            action(
                "document-edit-symbolic",
                &tr("tree-action-rename"),
                s,
                req(TreeRequest::Rename { node: r.node }),
            ),
            action(
                "edit-delete-symbolic",
                &tr("tree-action-delete"),
                s,
                req(TreeRequest::Delete { node: r.node }),
            ),
        ],
    };
    for b in &items {
        actions.append(b);
    }
    actions
}

pub(crate) fn connect_row(
    send: &Sender,
    quiet: &Rc<Cell<bool>>,
    root: &gtk::Box,
    actions: &gtk::Box,
    node: NodeId,
) {
    let send_focus = send.clone();
    let click = gtk::GestureClick::new();
    let click_send = send.clone();
    click.connect_pressed(move |_, _, _, _| {
        click_send(TreeMsg::Focus(node));
        match node {
            NodeId::Folder(id) => click_send(TreeMsg::ToggleFolder(id)),
            NodeId::Session(id) => click_send(TreeMsg::Select(id)),
        }
    });
    root.add_controller(click);
    let (hover, focus_inside) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let show = {
        let (actions, hover, focus_inside) = (actions.clone(), hover.clone(), focus_inside.clone());
        move || actions.set_visible(hover.get() || focus_inside.get())
    };
    let motion = gtk::EventControllerMotion::new();
    {
        let (hover, show) = (hover.clone(), show.clone());
        motion.connect_enter(move |_, _, _| {
            hover.set(true);
            show();
        });
    }
    {
        let show = show.clone();
        motion.connect_leave(move |_| {
            hover.set(false);
            show();
        });
    }
    root.add_controller(motion);
    let focus = gtk::EventControllerFocus::new();
    let quiet = quiet.clone();
    let focused_row = root.clone();
    focus.connect_contains_focus_notify(move |c| {
        focus_inside.set(c.contains_focus());
        show();
        if c.contains_focus() && focused_row.is_focus() && !quiet.get() {
            send_focus(TreeMsg::Focus(node));
        }
    });
    root.add_controller(focus);
    let keys = gtk::EventControllerKey::new();
    let send = send.clone();
    let row = root.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        // Enter on an action button is handled by the button; arrows only when focus is on the row itself
        if !row.is_focus() {
            return gtk::glib::Propagation::Proceed;
        }
        match nav_key(key) {
            Some(k) => {
                // this row received the key, so focus is on it even if the window has not reported it yet
                send(TreeMsg::Focus(node));
                send(TreeMsg::Key(k));
                gtk::glib::Propagation::Stop
            }
            None => gtk::glib::Propagation::Proceed,
        }
    });
    root.add_controller(keys);
}
