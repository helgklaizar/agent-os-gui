//! Application startup: process hardening, display, window, `--smoke` mode.

use gtk4::glib::ExitCode;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use core_client::blocking::BlockingClient;
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use ui::shell::{Shell, ShellOptions, present};
use ui::{hardening, prefs_io};

/// Application identifier (not specified in the documentation).
const APP_ID: &str = "ai.agentos.shell";
const SMOKE_LIMIT_SECS: u32 = 10;

pub fn run() -> ExitCode {
    // before threads and GTK: PR_SET_DUMPABLE=0 and GDK_DEBUG=no-portals
    if let Err(e) = hardening::apply() {
        eprintln!("agent-shell: process hardening failed: {e}");
        return ExitCode::from(2);
    }
    // log to stderr (collected by journald) with secret masking, level from RUST_LOG
    core_client::logging::init();
    let mut smoke = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--smoke" => smoke = true,
            other => {
                eprintln!("agent-shell: unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    if let Err(e) = adw::init() {
        eprintln!("agent-shell: display is not available: {e}");
        return ExitCode::from(1);
    }
    let Some(prefs_path) = prefs_io::default_path_from_env() else {
        eprintln!("agent-shell: cannot find the config directory (XDG_CONFIG_HOME or HOME)");
        return ExitCode::from(2);
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    // Shell holds the controllers and the bus: lives until the application exits
    let shell_slot: Rc<RefCell<Option<Shell>>> = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        if shell_slot.borrow().is_some() {
            return;
        }
        let shell = Shell::new(
            app,
            ShellOptions {
                prefs_path: prefs_path.clone(),
                core: core_from_env(),
                bar: ui::bar_rig::BarOptions {
                    client: bar_client_from_env(),
                    ..Default::default()
                },
            },
        );
        if smoke {
            quit_after_first_frame(app, shell.window());
        }
        present(shell.window());
        *shell_slot.borrow_mut() = Some(shell);
    });
    app.run_with_args::<&str>(&[])
}

fn quit_after_first_frame(app: &adw::Application, window: &adw::ApplicationWindow) {
    let done = Rc::new(Cell::new(false));
    let quit_app = app.clone();
    window.connect_map(move |window| {
        let (done, app) = (done.clone(), quit_app.clone());
        if let Some(clock) = window.frame_clock() {
            clock.connect_after_paint(move |_| {
                if !done.replace(true) {
                    app.quit();
                }
            });
        }
    });
    gtk::glib::timeout_add_seconds_local_once(SMOKE_LIMIT_SECS, || {
        eprintln!("agent-shell: --smoke: no first frame in {SMOKE_LIMIT_SECS} s");
        std::process::exit(1);
    });
}

/// Core client. In the product the token arrives as a descriptor from `agentd gui-launch`: until it is there,
/// the GUI works without a connection and the theme comes from the cache. For manual work with `fake-agentd`, build with the `test-token-file` feature.
#[cfg(feature = "test-token-file")]
fn core_from_env() -> Option<Arc<BlockingClient>> {
    use core_client::{Endpoint, TokenSource};
    let source =
        TokenSource::from_test_env_value(std::env::var("AGENTD_TOKEN_FILE").ok().as_deref())?;
    let token = source
        .read()
        .inspect_err(|e| eprintln!("agent-shell: token not read: {e}"))
        .ok()?;
    let Some(endpoint) = Endpoint::from_env() else {
        eprintln!(
            "agent-shell: core address is not set (AGENTD_SOCKET, AGENTD_ADDR or XDG_RUNTIME_DIR)"
        );
        return None;
    };
    BlockingClient::new(endpoint, token)
        .inspect_err(|e| eprintln!("agent-shell: core client not created: {e}"))
        .ok()
        .map(Arc::new)
}

#[cfg(not(feature = "test-token-file"))]
fn core_from_env() -> Option<Arc<BlockingClient>> {
    None
}

/// Core client with WS for the bar: the same token and address source as `core_from_env`.
#[cfg(feature = "test-token-file")]
fn bar_client_from_env() -> Option<Arc<core_client::CoreClient>> {
    use core_client::{CoreClient, CoreConfig, Endpoint, TokenSource};
    let source =
        TokenSource::from_test_env_value(std::env::var("AGENTD_TOKEN_FILE").ok().as_deref())?;
    let endpoint = Endpoint::from_env()?;
    CoreClient::start(CoreConfig::new(endpoint, source))
        .inspect_err(|e| eprintln!("agent-shell: bar client not started: {e}"))
        .ok()
        .map(Arc::new)
}

#[cfg(not(feature = "test-token-file"))]
fn bar_client_from_env() -> Option<Arc<core_client::CoreClient>> {
    None
}
