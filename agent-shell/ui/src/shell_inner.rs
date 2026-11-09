//! Shell window internals: controllers, bus, `ui.json` and the bridge to widgets.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use controllers::controllers::shell::{ShellController, ShellMsg};
use controllers::controllers::theme::{ThemeController, ThemeMsg};
use controllers::i18n::{tr, tr_args};
use controllers::{
    BusEvent, Column, Controller, CoreCommand, Effect, EventBus, ResolvedTheme, Theme,
};
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::bar_rig::BarRig;
use crate::chat_rig::ChatRig;
use crate::core_worker::CoreWorker;
use crate::notif_rig::NotifRig;
use crate::notif_toast::ToastView;
use crate::prefs_io::PrefsStore;
use crate::settings_rig::SettingsRig;
use crate::shell::ColumnId;
use crate::shell_view::{Bar, Column as ColumnView};
use crate::theme_tokens::class_name;
use crate::tools_rig::ToolsRig;
use crate::tree_rig::TreeRig;

/// How long after the last window resize `ui.json` is written.
const SIZE_FLUSH_DELAY: Duration = Duration::from_millis(500);

pub(crate) struct Inner {
    pub(crate) window: adw::ApplicationWindow,
    pub(crate) bar: Bar,
    /// Order: notifications, sessions, chat, tools.
    pub(crate) columns: [ColumnView; 4],
    pub(crate) shell: RefCell<ShellController>,
    pub(crate) theme: RefCell<ThemeController>,
    pub(crate) bus: EventBus,
    pub(crate) bar_rig: Rc<BarRig>,
    pub(crate) tree_rig: Rc<TreeRig>,
    pub(crate) chat_rig: Rc<ChatRig>,
    pub(crate) notif_rig: Rc<NotifRig>,
    pub(crate) tools_rig: Rc<ToolsRig>,
    pub(crate) settings_rig: Rc<SettingsRig>,
    pub(crate) toast: Rc<ToastView>,
    pub(crate) toast_ticking: Cell<bool>,
    pub(crate) prefs: RefCell<PrefsStore>,
    pub(crate) worker: Option<CoreWorker>,
    pub(crate) flush_scheduled: Cell<bool>,
    pub(crate) size_watch: Cell<bool>,
    pub(crate) last_size: Cell<(i32, i32)>,
}

pub(crate) fn theme_name(theme: Theme) -> String {
    match theme {
        Theme::Auto => tr("theme-auto"),
        Theme::Dark => tr("theme-dark"),
        Theme::Light => tr("theme-light"),
    }
}

pub(crate) fn next_theme(theme: Theme) -> Theme {
    match theme {
        Theme::Auto => Theme::Dark,
        Theme::Dark => Theme::Light,
        Theme::Light => Theme::Auto,
    }
}

impl Inner {
    pub(crate) fn column(&self, id: ColumnId) -> &ColumnView {
        let index = match id {
            ColumnId::Notifications => 0,
            ColumnId::Sessions => 1,
            ColumnId::Chat => 2,
            ColumnId::Tools => 3,
        };
        &self.columns[index]
    }

    pub(crate) fn connect(self: &Rc<Self>, style: &adw::StyleManager) {
        for (id, column) in [
            (ColumnId::Notifications, Column::Notifications),
            (ColumnId::Sessions, Column::Sessions),
            (ColumnId::Tools, Column::Tools),
        ] {
            let weak = Rc::downgrade(self);
            if let Some(button) = &self.column(id).collapse {
                button.connect_clicked(move |_| {
                    if let Some(inner) = weak.upgrade() {
                        inner.send_shell(ShellMsg::ToggleColumn(column));
                    }
                });
            }
        }
        let weak = Rc::downgrade(self);
        self.bar.theme_button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                let next = next_theme(inner.theme.borrow().view().selected);
                inner.send_theme(ThemeMsg::UserSelected(next));
            }
        });
        let weak = Rc::downgrade(self);
        style.connect_dark_notify(move |style| {
            if let Some(inner) = weak.upgrade()
                && inner.theme.borrow().view().selected == Theme::Auto
            {
                inner.send_theme(ThemeMsg::SystemChanged {
                    dark: style.is_dark(),
                });
            }
        });
        let weak = Rc::downgrade(self);
        self.bus.subscribe(move |event| {
            let Some(inner) = weak.upgrade() else { return };
            match event {
                BusEvent::LayoutChanged => inner.apply_layout(),
                BusEvent::ThemeChanged(_) => inner.apply_theme(),
                BusEvent::SelectTheme(theme) => inner.send_theme(ThemeMsg::UserSelected(*theme)),
                BusEvent::LanguageChanged(lang) => controllers::i18n::set_language(*lang),
                BusEvent::FocusRestore(target) => inner.tree_rig.restore_focus(*target),
                BusEvent::ExpandColumn(_) => inner.send_shell(ShellMsg::Bus(event.clone())),
                BusEvent::ShowToast { .. } => {
                    inner.send_shell(ShellMsg::Bus(event.clone()));
                    inner.apply_toast();
                }
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.flush();
            }
            gtk::glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.window.connect_map(move |_| {
            if let Some(inner) = weak.upgrade() {
                Inner::watch_size(&inner);
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_realize(move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            let weak = weak.clone();
            surface.connect_layout(move |_, _, _| {
                if let Some(inner) = weak.upgrade() {
                    Inner::watch_size(&inner);
                }
            });
        });
    }

    pub(crate) fn send_shell(self: &Rc<Self>, msg: ShellMsg) {
        let effects = self.shell.borrow_mut().update(msg, Instant::now());
        self.run(effects);
    }

    pub(crate) fn send_theme(self: &Rc<Self>, msg: ThemeMsg) {
        let effects = self.theme.borrow_mut().update(msg, Instant::now());
        self.run(effects);
    }

    pub(crate) fn run(self: &Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Publish(event) => self.bus.publish(event),
                Effect::Command(CoreCommand::PatchTheme(theme)) => {
                    if let Some(worker) = &self.worker {
                        worker.patch(theme);
                    }
                }
                // Logout and About are run by the bar (`BarRig`), tree commands by `TreeRig`; the theme has no such commands
                Effect::Command(_) => {}
                Effect::Persist(prefs) => {
                    self.prefs.borrow_mut().apply(&prefs);
                    if prefs.theme.is_none() && prefs.columns.is_none() {
                        self.schedule_flush();
                    } else {
                        self.flush();
                    }
                }
            }
        }
    }

    pub(crate) fn flush(&self) {
        if let Err(e) = self.prefs.borrow_mut().flush() {
            gtk::glib::g_warning!("agent-shell", "ui.json not written: {e}");
        }
    }

    /// The window size changes often (dragging): write once after a pause.
    pub(crate) fn schedule_flush(self: &Rc<Self>) {
        if self.flush_scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_local_once(SIZE_FLUSH_DELAY, move || {
            if let Some(inner) = weak.upgrade() {
                inner.flush_scheduled.set(false);
                inner.flush();
            }
        });
    }

    pub(crate) fn apply_layout(&self) {
        let layout = self.shell.borrow().view().layout;
        let sizes = [
            (
                ColumnId::Notifications,
                layout.notifications_w,
                layout.notifications_collapsed,
            ),
            (
                ColumnId::Sessions,
                layout.sessions_w,
                layout.sessions_collapsed,
            ),
            (ColumnId::Tools, layout.tools_w, layout.tools_collapsed),
        ];
        for (id, width, collapsed) in sizes {
            let column = self.column(id);
            column.root.set_size_request(width as i32, -1);
            column.set_collapsed(collapsed);
            match id {
                ColumnId::Sessions => self.tree_rig.collapsed(collapsed),
                ColumnId::Notifications => self.notif_rig.collapsed(collapsed),
                ColumnId::Tools => self.tools_rig.collapsed(collapsed),
                ColumnId::Chat => {}
            }
        }
    }

    /// Shows a toast from `ShellController`; while it is visible, a `Tick` fires every 250 ms (the controller decides the 4 s timeout).
    pub(crate) fn apply_toast(self: &Rc<Self>) {
        let toast = self.shell.borrow().view().toast;
        self.toast.apply(toast.as_ref());
        if toast.is_none() || self.toast_ticking.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(inner) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            inner.send_shell(ShellMsg::Tick);
            let shown = inner.shell.borrow().view().toast;
            inner.toast.apply(shown.as_ref());
            if shown.is_some() {
                gtk::glib::ControlFlow::Continue
            } else {
                inner.toast_ticking.set(false);
                gtk::glib::ControlFlow::Break
            }
        });
    }

    pub(crate) fn apply_theme(self: &Rc<Self>) {
        let style = adw::StyleManager::default();
        let selected = self.theme.borrow().view().selected;
        style.set_color_scheme(match selected {
            Theme::Auto => adw::ColorScheme::Default,
            Theme::Dark => adw::ColorScheme::ForceDark,
            Theme::Light => adw::ColorScheme::ForceLight,
        });
        if selected == Theme::Auto {
            self.send_theme(ThemeMsg::SystemChanged {
                dark: style.is_dark(),
            });
        }
        let view = self.theme.borrow().view();
        for resolved in [ResolvedTheme::Dark, ResolvedTheme::Light] {
            self.window.remove_css_class(class_name(resolved));
        }
        self.window.add_css_class(class_name(view.resolved));
        let label = tr_args("bar-theme-button", &[("theme", &theme_name(view.selected))]);
        self.bar.theme_button.set_label(&label);
    }
}
