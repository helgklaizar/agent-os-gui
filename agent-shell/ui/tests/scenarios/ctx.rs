//! Shared by the scenarios: compositor, runtime for `fake-agentd`, GTK and wait helpers.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use ui::shell::{Shell, ShellOptions};

use crate::weston::Weston;

pub type Scenario = fn(&Ctx) -> Result<(), String>;

pub struct Ctx {
    pub weston: Weston,
    /// The runtime `fake-agentd` lives on.
    pub rt: tokio::runtime::Runtime,
    pub app: adw::Application,
    /// `XDG_DATA_HOME` of the scenarios: browser profiles do not land in the home folder.
    pub data: tempfile::TempDir,
}

impl Ctx {
    #[allow(unsafe_code)]
    pub fn new() -> Ctx {
        let weston = Weston::start(2000, 1000);
        let data = tempfile::tempdir().expect("scenario data directory");
        // GTK reads the environment at initialisation: set the compositor and backend before it, there are no threads yet
        // SAFETY: called from main before threads are created (the tokio runtime is created below).
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", weston.runtime_dir());
            std::env::set_var("WAYLAND_DISPLAY", weston.socket());
            std::env::set_var("GDK_BACKEND", "wayland");
            std::env::set_var("GSK_RENDERER", "cairo");
            std::env::set_var("GDK_DEBUG", "no-portals");
            std::env::set_var("XDG_DATA_HOME", data.path());
            // without an accessibility bus GTK writes CRITICAL; the accessibility tree scenario enables AT-SPI itself
            if std::env::var_os("GTK_A11Y").is_none() {
                std::env::set_var("GTK_A11Y", "none");
            }
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        adw::init().expect("GTK and libadwaita did not initialise (no compositor?)");
        let app = adw::Application::builder()
            .application_id("ai.agentos.shell.scenarios")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>)
            .expect("application registration");
        Ctx {
            weston,
            rt,
            app,
            data,
        }
    }

    /// Closes windows left over from a scenario: live windows of earlier scenarios accumulate layout and timers and slow the next ones.
    pub fn close_windows(&self) {
        for window in self.app.windows() {
            window.destroy();
        }
        let main = gtk::glib::MainContext::default();
        while main.iteration(false) {}
    }

    pub fn shell(&self, prefs: PathBuf) -> Shell {
        Shell::new(
            &self.app,
            ShellOptions {
                prefs_path: prefs,
                core: None,
                bar: ui::bar_rig::BarOptions::default(),
            },
        )
    }
}

/// Opens a session like a click in the tree: the tools column is visible only with an open session.
pub fn open_session(shell: &Shell, session_id: i64) {
    shell.bus().publish(controllers::BusEvent::OpenSession {
        session_id,
        tab: controllers::Tab::Chat,
    });
    let main = gtk::glib::MainContext::default();
    while main.iteration(false) {}
}

/// Spins the GTK main loop until the condition holds; otherwise `Err` with a description.
pub fn settle(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let ctx = gtk::glib::MainContext::default();
    loop {
        while ctx.iteration(false) {}
        if cond() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for: {what}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub const WAIT: Duration = Duration::from_secs(5);
