//! Bar and dialogs in real GTK under headless Wayland: About, menu events.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use controllers::{BusEvent, MenuItem, SettingsPage};
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreConfig, Endpoint, Token, TokenSource};
use fake_agentd::{Config, FakeAgentd};
use gtk4 as gtk;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use ui::bar_rig::BarOptions;
use ui::shell::{Shell, ShellOptions};

use crate::ctx::{Ctx, Scenario as Run, WAIT, settle};

pub fn scenarios() -> Vec<(&'static str, Run)> {
    vec![
        ("ui_bar_about_dialog", about_dialog),
        ("ui_bar_dialog_closes_by_backdrop_click", backdrop_click),
        ("ui_bar_menu_events", menu_events),
    ]
}

/// Core mock, client and a shell with a bar connected to the core.
pub(crate) struct Rig {
    pub(crate) d: FakeAgentd,
    pub(crate) shell: Shell,
    _dir: tempfile::TempDir,
    _client: Arc<CoreClient>,
}

pub(crate) fn fixed_clock() -> i64 {
    1_793_628_309 // 2026-11-02 14:05:09 UTC
}

pub(crate) fn open(
    ctx: &Ctx,
    config: Config,
    bar: impl FnOnce(&mut BarOptions),
) -> Result<Rig, String> {
    open_w(ctx, config, 1280, bar)
}

pub(crate) fn open_w(
    ctx: &Ctx,
    config: Config,
    width: i32,
    bar: impl FnOnce(&mut BarOptions),
) -> Result<Rig, String> {
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(config))
        .map_err(|e| e.to_string())?;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let endpoint = Endpoint::Unix(d.socket_path().to_path_buf());
    let token = Token::new(d.admin_token());
    let core = BlockingClient::new(endpoint.clone(), token.clone()).map_err(|e| e.to_string())?;
    let mut config = CoreConfig::new(endpoint, TokenSource::Value(token));
    config.ping_interval = std::time::Duration::from_secs(60);
    let client = Arc::new(CoreClient::start(config).map_err(|e| e.to_string())?);
    let mut options = BarOptions::default();
    bar(&mut options);
    options.client = Some(client.clone());
    let shell = Shell::new(
        &ctx.app,
        ShellOptions {
            prefs_path: dir.path().join("agent-shell/ui.json"),
            core: Some(Arc::new(core)),
            bar: options,
        },
    );
    shell.window().set_default_size(width, 720);
    shell.window().present();
    Ok(Rig {
        d,
        shell,
        _dir: dir,
        _client: client,
    })
}

/// All `gtk::Label` captions in the subtree.
pub(crate) fn texts(widget: &gtk::Widget) -> Vec<String> {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if let Ok(l) = w.clone().downcast::<gtk::Label>() {
            out.push(l.text().to_string());
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            walk(&x, out);
            c = x.next_sibling();
        }
    }
    let mut out = Vec::new();
    walk(widget, &mut out);
    out
}

pub(crate) fn bar_texts(shell: &Shell) -> Vec<String> {
    texts(&shell.bar())
}

pub(crate) fn has_text(shell: &Shell, part: &str) -> bool {
    bar_texts(shell).iter().any(|t| t.contains(part))
}

pub(crate) fn wait_loaded(rig: &Rig) -> Result<(), String> {
    settle("quotas and metrics in the bar", WAIT, || {
        has_text(&rig.shell, "38%") && has_text(&rig.shell, "Disk") && has_text(&rig.shell, "24°")
    })
}

/// Presses Esc the way GTK does: with the `key-pressed` signal of the window's key controller.
pub(crate) fn press_escape(shell: &Shell) -> Result<(), String> {
    let model = shell.window().observe_controllers();
    for i in 0..model.n_items() {
        let Some(obj) = model.item(i) else { continue };
        if let Ok(keys) = obj.downcast::<gtk::EventControllerKey>() {
            let key = gtk::gdk::Key::Escape.into_glib();
            let handled: bool = keys.emit_by_name(
                "key-pressed",
                &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
            );
            return if handled {
                Ok(())
            } else {
                Err("Esc not handled".into())
            };
        }
    }
    Err("the window has no key controller".into())
}

pub(crate) fn open_menu_and_click(shell: &Shell, item: MenuItem) -> Result<(), String> {
    shell.menu_button().emit_clicked();
    settle("menu open", WAIT, || shell.menu_open())?;
    let button = shell.menu_item(item).ok_or("no menu item")?;
    button.emit_clicked();
    Ok(())
}

pub(crate) fn focus_on_menu_button(shell: &Shell) -> bool {
    let menu = shell.menu_button();
    GtkWindowExt::focus(shell.window())
        .is_some_and(|f| f == *menu.upcast_ref::<gtk::Widget>() || f.is_ancestor(&menu))
}

/// Presses a button the way a user does: the `clicked` signal.
pub(crate) fn click(button: &gtk::Button) {
    button.emit_clicked();
}

fn about_dialog(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |_| {})?;
    wait_loaded(&rig)?;
    open_menu_and_click(&rig.shell, MenuItem::About)?;
    settle("About open", WAIT, || rig.shell.dialog_open())?;
    settle("core version in About", WAIT, || {
        rig.shell.dialog_texts().iter().any(|t| t == "0.0.0-fake")
    })?;
    let shown = rig.shell.dialog_texts();
    for want in ["About Agent OS", "Ubuntu 26.04 · Wayland", "~/.agentos/"] {
        if !shown.iter().any(|t| t == want) {
            return Err(format!("About has no '{want}': {shown:?}"));
        }
    }
    press_escape(&rig.shell)?;
    settle("About closed by Esc", WAIT, || !rig.shell.dialog_open())?;
    settle("focus returned to the menu", WAIT, || {
        focus_on_menu_button(&rig.shell)
    })?;
    // without a dialog Esc handles nothing
    if press_escape(&rig.shell).is_ok() {
        return Err("Esc without a dialog must not be intercepted".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn backdrop_click(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |_| {})?;
    wait_loaded(&rig)?;
    open_menu_and_click(&rig.shell, MenuItem::About)?;
    settle("About open", WAIT, || rig.shell.dialog_open())?;
    let layer = rig.shell.dialog_layer();
    settle("layer stretched", WAIT, || layer.width() > 400)?;
    let model = layer.observe_controllers();
    let click = (0..model.n_items())
        .filter_map(|i| model.item(i))
        .find_map(|o| o.downcast::<gtk::GestureClick>().ok())
        .ok_or("the layer has no click gesture")?;
    // a click inside the card does not close it
    let card_x = f64::from(layer.width()) / 2.0;
    let card_y = f64::from(layer.height()) / 2.0;
    click.emit_by_name::<()>("pressed", &[&1i32, &card_x, &card_y]);
    for _ in 0..5 {
        gtk::glib::MainContext::default().iteration(false);
    }
    if !rig.shell.dialog_open() {
        return Err("a click on the card closed the dialog".into());
    }
    click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    settle("About closed by a click on the backdrop", WAIT, || {
        !rig.shell.dialog_open()
    })?;
    settle("focus returned to the menu", WAIT, || {
        focus_on_menu_button(&rig.shell)
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn menu_events(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |_| {})?;
    wait_loaded(&rig)?;
    let seen: Rc<RefCell<Vec<BusEvent>>> = Rc::default();
    let sink = seen.clone();
    rig.shell
        .bus()
        .subscribe(move |e| sink.borrow_mut().push(e.clone()));
    open_menu_and_click(&rig.shell, MenuItem::User)?;
    settle("OpenSettings(Account)", WAIT, || {
        seen.borrow()
            .contains(&BusEvent::OpenSettings(SettingsPage::Account))
    })?;
    open_menu_and_click(&rig.shell, MenuItem::Settings)?;
    settle("OpenSettings(Root)", WAIT, || {
        seen.borrow()
            .contains(&BusEvent::OpenSettings(SettingsPage::Root))
    })?;
    // a click on a quota icon opens Settings -> Models
    let q = rig
        .d
        .db()
        .reader()
        .and_then(|c| {
            Ok(
                c.query_row("SELECT id FROM accounts WHERE slug = 'cl1'", [], |x| {
                    x.get::<_, i64>(0)
                })?,
            )
        })
        .map_err(|e| e.to_string())?;
    let button = rig.shell.quota_button(q).ok_or("no quota icon cl1")?;
    button.emit_clicked();
    settle("OpenSettings(Models)", WAIT, || {
        seen.borrow()
            .contains(&BusEvent::OpenSettings(SettingsPage::Models))
    })?;
    if rig.shell.dialog_open() {
        return Err("User, Settings and quota do not open bar dialogs".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
