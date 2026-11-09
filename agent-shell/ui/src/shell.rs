//! Shell window: the bar and four columns, themes. Controllers decide the logic; this file has only widgets and the bridge.
//!
//! Data flow: widget signal -> message to the controller -> `Effect` -> bus (`Publish`), `ui.json` (`Persist`),
//! core worker thread (`Command`) -> bus event -> widget update.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::controllers::shell::ShellController;
use controllers::controllers::theme::{ThemeController, ThemeMsg};
use controllers::i18n::tr;
use controllers::timezone::system_zone;
use controllers::{BusEvent, EventBus, Tab};
use core_client::blocking::BlockingClient;
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::bar_rig::{BarOptions, BarRig};
use crate::chat_rig::ChatRig;
use crate::core_worker::{CoreWorker, Reply};
use crate::notif_rig::NotifRig;
use crate::notif_toast::ToastView;
use crate::prefs_io::PrefsStore;
use crate::settings_rig::SettingsRig;
use crate::shell_inner::Inner;
use crate::shell_view::{Bar, Column as ColumnView};
use crate::theme_tokens::css;
use crate::tools_rig::{ToolsOptions, ToolsRig};
use crate::tree_rig::TreeRig;

/// Minimum window size (spec section 4): a single value, `controllers::layout::MIN_WINDOW`.
const MIN_SIZE: (i32, i32) = (
    controllers::layout::MIN_WINDOW.0 as i32,
    controllers::layout::MIN_WINDOW.1 as i32,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnId {
    Notifications,
    Sessions,
    Chat,
    Tools,
}

pub struct ShellOptions {
    pub prefs_path: PathBuf,
    /// Core client; `None` means working without a connection, theme from the `ui.json` cache.
    pub core: Option<Arc<BlockingClient>>,
    /// Bar: core client for frames, time zone and clock.
    pub bar: BarOptions,
}

pub struct Shell {
    pub(crate) inner: Rc<Inner>,
}

fn install_css() {
    thread_local!(static INSTALLED: Cell<bool> = const { Cell::new(false) });
    if INSTALLED.replace(true) {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css());
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

impl Shell {
    pub fn new(app: &adw::Application, options: ShellOptions) -> Shell {
        install_css();
        let config_path = options.prefs_path.display().to_string();
        let (store, warning) = PrefsStore::open(options.prefs_path);
        if let Some(reason) = warning {
            gtk::glib::g_warning!("agent-shell", "{reason}");
        }
        let file = *store.file();
        let style = adw::StyleManager::default();
        // the system scheme, not the previous window's forced one: read it before any changes
        style.set_color_scheme(adw::ColorScheme::Default);

        let window = adw::ApplicationWindow::new(app);
        window.set_title(Some(&tr("app-name")));
        window.set_size_request(MIN_SIZE.0, MIN_SIZE.1);
        window.set_default_size(file.window.w as i32, file.window.h as i32);
        window.add_css_class("shell");

        let bar = Bar::new();
        let columns = [
            ColumnView::new(ColumnId::Notifications),
            ColumnView::new(ColumnId::Sessions),
            ColumnView::new(ColumnId::Chat),
            ColumnView::new(ColumnId::Tools),
        ];
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_vexpand(true);
        for c in &columns {
            row.append(&c.root);
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&bar.root);
        root.append(&row);
        // dialogs (About, Logout) sit in a layer above the whole window
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&root));
        let bus = EventBus::new();
        let client = options.bar.client.clone();
        let zone = options.bar.zone.clone().unwrap_or_else(|| system_zone().0);
        let bar_rig = BarRig::new(
            bar.widgets.clone(),
            &window,
            bus.clone(),
            options.bar,
            options.core.clone(),
        );
        let sessions = &columns[1];
        let tree_rig = TreeRig::new(
            &sessions.body,
            sessions.badge.clone(),
            bus.clone(),
            options.core.clone(),
            client.as_ref(),
        );
        let chat_rig = ChatRig::new(
            &columns[2].body,
            bus.clone(),
            options.core.clone(),
            client.as_ref(),
        );
        let title_window = window.clone();
        let notif_rig = NotifRig::new(
            &columns[0].body,
            columns[0].badge.clone(),
            bus.clone(),
            options.core.clone(),
            client.as_ref(),
            zone,
            Rc::new(move |title: &str| title_window.set_title(Some(title))),
        );
        let move_tree = Rc::downgrade(&tree_rig);
        let tools_column = columns[3].root.clone();
        let tools_rig = ToolsRig::new(
            &columns[3].body,
            bus.clone(),
            options.core.clone(),
            ToolsOptions {
                data_dir: gtk::glib::user_data_dir()
                    .join("agent-shell")
                    .join("browser"),
                home: gtk::glib::home_dir(),
            },
            Rc::new(move |session: i64| {
                if let Some(rig) = move_tree.upgrade() {
                    rig.send(TreeMsg::Request(TreeRequest::Move { session }));
                }
            }),
            Rc::new(move |visible: bool| tools_column.set_visible(visible)),
        );
        let settings_rig = SettingsRig::new(bus.clone(), options.core.clone(), config_path);
        let settings_tap = Rc::downgrade(&settings_rig);
        let tools_tap = Rc::downgrade(&tools_rig);
        let notif_tap = Rc::downgrade(&notif_rig);
        let toast_bus = bus.clone();
        let toast = ToastView::new(Rc::new(move |session_id: i64| {
            toast_bus.publish(BusEvent::OpenSession {
                session_id,
                tab: Tab::Chat,
            });
        }));
        let chat_tap = Rc::downgrade(&chat_rig);
        let sink_rig = Rc::downgrade(&tree_rig);
        bar_rig.set_command_sink(Rc::new(move |command| {
            if let Some(rig) = sink_rig.upgrade() {
                rig.submit(command);
            }
        }));
        let tap_rig = Rc::downgrade(&tree_rig);
        bar_rig.add_tap(Rc::new(move |event| {
            if let Some(rig) = tap_rig.upgrade() {
                rig.on_event(event);
            }
        }));
        bar_rig.add_tap(Rc::new(move |event| {
            if let Some(rig) = notif_tap.upgrade() {
                rig.on_event(event);
            }
        }));
        bar_rig.add_tap(Rc::new(move |event| {
            if let Some(rig) = chat_tap.upgrade() {
                rig.on_event(event);
            }
        }));
        bar_rig.add_tap(Rc::new(move |event| {
            if let Some(rig) = tools_tap.upgrade() {
                rig.on_event(event);
            }
        }));
        bar_rig.add_tap(Rc::new(move |event| {
            if let Some(rig) = settings_tap.upgrade() {
                rig.on_event(event);
            }
        }));
        overlay.add_overlay(&bar.widgets.menu_layer);
        overlay.add_overlay(&settings_rig.dialog.layer);
        overlay.add_overlay(&bar_rig.host.layer);
        overlay.add_overlay(&toast.root);
        window.set_content(Some(&overlay));

        let (worker, replies) = match options.core {
            Some(core) => {
                let (w, r) = CoreWorker::start(core);
                (Some(w), Some(r))
            }
            None => (None, None),
        };
        let inner = Rc::new(Inner {
            window,
            bar,
            columns,
            shell: RefCell::new(ShellController::new(file)),
            theme: RefCell::new(ThemeController::new(style.is_dark())),
            bus,
            bar_rig,
            tree_rig,
            chat_rig,
            notif_rig,
            tools_rig,
            settings_rig,
            toast,
            toast_ticking: Cell::new(false),
            prefs: RefCell::new(store),
            worker,
            flush_scheduled: Cell::new(false),
            size_watch: Cell::new(false),
            last_size: Cell::new((0, 0)),
        });
        inner.connect(&style);
        // any change of content size, including an already shown window, reaches the controller
        let probe = gtk::DrawingArea::new();
        probe.set_hexpand(true);
        probe.set_vexpand(true);
        probe.set_can_target(false);
        probe.set_can_focus(false);
        probe.set_accessible_role(gtk::AccessibleRole::Presentation);
        let weak = Rc::downgrade(&inner);
        probe.connect_resize(move |_, w, h| {
            let weak = weak.clone();
            // the size is applied outside the layout pass: columns change size_request
            gtk::glib::idle_add_local_once(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.on_content_size(w, h);
                }
            });
        });
        overlay.add_overlay(&probe);
        inner.send_theme(ThemeMsg::CacheLoaded(file.theme));
        inner.apply_layout();
        inner.apply_theme();
        if let Some(replies) = replies {
            let weak = Rc::downgrade(&inner);
            gtk::glib::spawn_future_local(async move {
                while let Ok(reply) = replies.recv().await {
                    let Some(inner) = weak.upgrade() else { break };
                    match reply {
                        Reply::CoreTheme(theme) => inner.send_theme(ThemeMsg::CoreValue(theme)),
                        Reply::PatchFailed(e) => {
                            gtk::glib::g_warning!("agent-shell", "theme not saved in core: {e}");
                        }
                    }
                }
            });
        }
        Shell { inner }
    }

    /// Write `ui.json` now.
    pub fn flush_prefs(&self) {
        self.inner.flush();
    }
}

/// Show the window: `present()` first, then `fullscreen()` (cage 0.2.1 crashes in the reverse order).
pub fn present(window: &adw::ApplicationWindow) {
    window.present();
    window.fullscreen();
}
