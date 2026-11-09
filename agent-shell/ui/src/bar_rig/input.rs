//! Bar input subscriptions: menu, bus, Esc and the one-second tick.

use std::rc::Rc;

use controllers::controllers::bar::BarMsg;
use controllers::controllers::dialog::DialogMsg;
use controllers::{BusEvent, Controller, FocusTarget};
use gtk4 as gtk;
use gtk4::prelude::*;

use super::BarRig;

impl BarRig {
    pub(super) fn connect(self: &Rc<Self>) {
        self.connect_menu();
        self.connect_bus();
        self.connect_keys();
        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
            Some(rig) => {
                rig.tick();
                gtk::glib::ControlFlow::Continue
            }
            None => gtk::glib::ControlFlow::Break,
        });
    }

    fn connect_menu(self: &Rc<Self>) {
        let w = &self.widgets;
        let entries = self.cluster.borrow().bar.view().menu;
        for entry in entries {
            let button = gtk::Button::builder()
                .label(&entry.label)
                .accessible_role(gtk::AccessibleRole::MenuItem)
                .build();
            button.add_css_class("flat");
            button.add_css_class("menu-item");
            crate::a11y::set_name(&button, &entry.label);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(rig) = weak.upgrade() {
                    rig.bar_msg(BarMsg::MenuSelect(entry.item));
                }
            });
            w.menu_card.append(&button);
            w.menu_items.borrow_mut().push((entry.item, button));
        }
        let weak = Rc::downgrade(self);
        w.menu_button.connect_clicked(move |_| {
            if let Some(rig) = weak.upgrade() {
                rig.bar_msg(BarMsg::MenuToggle);
            }
        });
        // a click on the layer outside the card closes the menu
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(rig) = weak.upgrade() else { return };
            let layer = &rig.widgets.menu_layer;
            if layer
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|hit| hit == *layer.upcast_ref::<gtk::Widget>())
            {
                rig.bar_msg(BarMsg::MenuDismiss);
            }
        });
        w.menu_layer.add_controller(click);
    }

    fn connect_bus(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.bus.subscribe(move |event| {
            let Some(rig) = weak.upgrade() else { return };
            match event {
                BusEvent::OpenDialog { kind, opener } => {
                    rig.send_dialog(DialogMsg::Open {
                        kind: *kind,
                        opener: *opener,
                    });
                }
                BusEvent::OpenForm { form, opener } => {
                    rig.send_dialog(DialogMsg::OpenForm {
                        form: form.clone(),
                        opener: *opener,
                    });
                }
                BusEvent::FocusRestore(FocusTarget::MenuItem(_)) => {
                    // menu items are hidden together with the popover: focus returns to the menu button
                    rig.widgets.menu_button.grab_focus();
                }
                _ => {}
            }
        });
    }

    /// Esc closes the top dialog, or the menu when there is no dialog (before the rest of the window sees it).
    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(rig) = weak.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            if key != gtk::gdk::Key::Escape {
                return gtk::glib::Propagation::Proceed;
            }
            if rig.host.is_open() {
                rig.send_dialog(DialogMsg::Esc);
                return gtk::glib::Propagation::Stop;
            }
            if rig.widgets.menu_layer.is_visible() {
                rig.bar_msg(BarMsg::MenuDismiss);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
    }
}
