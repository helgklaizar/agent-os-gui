//! Window size tracking: recomputes the layout and the Settings dialog height when the size changes.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use controllers::controllers::shell::ShellMsg;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::shell_inner::Inner;

impl Inner {
    /// Content size changed: recomputes the layout if the window became different.
    pub(crate) fn on_content_size(self: &Rc<Self>, w: i32, h: i32) {
        let size = (self.window.width(), self.window.height());
        let size = if size.0 > 0 && size.1 > 0 {
            size
        } else {
            (w, h)
        };
        if size.0 > 0 && size.1 > 0 && size != self.last_size.get() {
            self.last_size.set(size);
            self.settings_rig.fit(size.1);
            self.send_shell(ShellMsg::WindowResized {
                w: size.0 as u32,
                h: size.1 as u32,
            });
        }
    }

    /// Tracks the window size while it changes: the frame tick lives until the size settles.
    pub(crate) fn watch_size(self: &Rc<Self>) {
        if self.size_watch.replace(true) {
            return;
        }
        let weak: Weak<Inner> = Rc::downgrade(self);
        let stable = Cell::new(0u32);
        self.window.add_tick_callback(move |window, _| {
            let Some(inner) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            let size = (window.width(), window.height());
            if size != inner.last_size.get() && size.0 > 0 && size.1 > 0 {
                inner.last_size.set(size);
                stable.set(0);
                inner.settings_rig.fit(size.1);
                inner.send_shell(ShellMsg::WindowResized {
                    w: size.0 as u32,
                    h: size.1 as u32,
                });
            } else {
                stable.set(stable.get() + 1);
            }
            if stable.get() >= 5 {
                inner.size_watch.set(false);
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    }
}
