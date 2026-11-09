//! "Sessions" column in real GTK under headless Wayland: keyboard. This is also the shared `Rig` stand.

use std::sync::Arc;
use std::time::Duration;

use controllers::tree_types::NodeId;
use core_client::blocking::BlockingClient;
use core_client::{CoreClient, CoreConfig, Endpoint, HttpClient, Token, TokenSource};
use fake_agentd::{Config, FakeAgentd};
use gtk4 as gtk;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use ui::bar_rig::BarOptions;
use ui::shell::{Shell, ShellOptions};

use crate::ctx::{Ctx, Scenario as Run, WAIT, settle};

pub fn scenarios() -> Vec<(&'static str, Run)> {
    vec![("ui_tree_keyboard_nav", keyboard_nav)]
}

pub(crate) struct Rig {
    pub(crate) d: FakeAgentd,
    pub(crate) shell: Shell,
    pub(crate) http: HttpClient,
    _dir: tempfile::TempDir,
    _client: Arc<CoreClient>,
}

pub(crate) fn open(ctx: &Ctx) -> Result<Rig, String> {
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(Config::temp().no_ticker()))
        .map_err(|e| e.to_string())?;
    open_with(ctx, d)
}

/// A wide window: all columns are expanded automatically (tools are visible without a manual expand).
pub(crate) fn open_wide(ctx: &Ctx) -> Result<Rig, String> {
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(Config::temp().no_ticker()))
        .map_err(|e| e.to_string())?;
    open_sized(ctx, d, 2000, 900)
}

pub(crate) fn open_with(ctx: &Ctx, d: FakeAgentd) -> Result<Rig, String> {
    open_sized(ctx, d, 1280, 720)
}

pub(crate) fn open_sized(ctx: &Ctx, d: FakeAgentd, w: i32, h: i32) -> Result<Rig, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let endpoint = Endpoint::Unix(d.socket_path().to_path_buf());
    let token = Token::new(d.admin_token());
    let core = BlockingClient::new(endpoint.clone(), token.clone()).map_err(|e| e.to_string())?;
    let http = HttpClient::new(endpoint.clone(), Some(token.clone()));
    let mut config = CoreConfig::new(endpoint, TokenSource::Value(token));
    config.ping_interval = Duration::from_secs(60);
    let client = Arc::new(CoreClient::start(config).map_err(|e| e.to_string())?);
    let shell = Shell::new(
        &ctx.app,
        ShellOptions {
            prefs_path: dir.path().join("agent-shell/ui.json"),
            core: Some(Arc::new(core)),
            bar: BarOptions {
                client: Some(client.clone()),
                ..BarOptions::default()
            },
        },
    );
    shell.window().set_default_size(w, h);
    shell.window().present();
    Ok(Rig {
        d,
        shell,
        http,
        _dir: dir,
        _client: client,
    })
}

impl Rig {
    pub(crate) fn rows(&self) -> usize {
        self.shell.tree_view().rows.len()
    }

    pub(crate) fn node(&self, name: &str) -> Option<NodeId> {
        self.shell
            .tree_view()
            .rows
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.node)
    }

    pub(crate) fn row_widget(&self, node: NodeId) -> Result<gtk::Box, String> {
        self.shell
            .tree()
            .row_widget(node)
            .ok_or_else(|| format!("no row widget {node:?}"))
    }

    pub(crate) fn call<T>(&self, ctx: &Ctx, f: impl std::future::Future<Output = T>) -> T {
        let _ = self;
        ctx.rt.block_on(f)
    }
}

pub(crate) fn wait_loaded(rig: &Rig) -> Result<(), String> {
    settle("tree from fixtures (10 rows)", WAIT, || rig.rows() == 10)
}

pub(crate) fn find_all<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Vec<T> {
    fn walk<T: IsA<gtk::Widget>>(w: &gtk::Widget, out: &mut Vec<T>) {
        if let Ok(t) = w.clone().downcast::<T>() {
            out.push(t);
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            walk(&x, out);
            c = x.next_sibling();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

pub(crate) fn labels(root: &impl IsA<gtk::Widget>) -> Vec<String> {
    find_all::<gtk::Label>(root.upcast_ref())
        .iter()
        .map(|l| l.text().to_string())
        .collect()
}

/// Presses a key the way GTK does: with the `key-pressed` signal of the widget's key controller.
pub(crate) fn press(widget: &impl IsA<gtk::Widget>, key: gtk::gdk::Key) -> Result<(), String> {
    let model = widget.observe_controllers();
    for i in 0..model.n_items() {
        let Some(obj) = model.item(i) else { continue };
        if let Ok(keys) = obj.downcast::<gtk::EventControllerKey>() {
            let handled: bool = keys.emit_by_name(
                "key-pressed",
                &[&key.into_glib(), &0u32, &gtk::gdk::ModifierType::empty()],
            );
            return if handled {
                Ok(())
            } else {
                Err(format!("key {key:?} not handled"))
            };
        }
    }
    Err("the widget has no key controller".into())
}

pub(crate) fn press_escape(shell: &Shell) -> Result<(), String> {
    press(shell.window(), gtk::gdk::Key::Escape)
}

pub(crate) fn tooltip_button(root: &gtk::Box, tip: &str) -> Result<gtk::Button, String> {
    find_all::<gtk::Button>(root.upcast_ref())
        .into_iter()
        .find(|b| b.tooltip_text().is_some_and(|t| t == tip))
        .ok_or_else(|| format!("no button '{tip}'"))
}

pub(crate) fn focus_is(widget: &gtk::Box) -> bool {
    widget.is_focus()
}

fn keyboard_nav(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let tree = rig.shell.tree();
    let first = rig.shell.tree_view().rows[0].node;
    // Tab lands on the single entry point, the first row
    let first_row = rig.row_widget(first)?;
    if !first_row.is_focusable() {
        return Err("the first row is not the Tab entry point".into());
    }
    first_row.grab_focus();
    let nodes: Vec<NodeId> = rig.shell.tree_view().rows.iter().map(|r| r.node).collect();
    press(&first_row, gtk::gdk::Key::Down)?;
    settle("Down: focus on the second row", WAIT, || {
        rig.row_widget(nodes[1]).is_ok_and(|w| focus_is(&w))
    })?;
    let focusable: Vec<bool> = nodes
        .iter()
        .map(|n| {
            rig.row_widget(*n)
                .map(|w| w.is_focusable())
                .unwrap_or(false)
        })
        .collect();
    if focusable.iter().filter(|f| **f).count() != 1 || !focusable[1] {
        return Err(format!("one Tab stop on the focused row: {focusable:?}"));
    }
    press(&rig.row_widget(nodes[1])?, gtk::gdk::Key::Up)?;
    settle("Up: focus on the first row", WAIT, || {
        rig.row_widget(first).is_ok_and(|w| focus_is(&w))
    })?;
    // Left collapses an expanded folder, Right expands it
    let before = rig.rows();
    press(&rig.row_widget(first)?, gtk::gdk::Key::Left)?;
    settle("Left collapsed the folder", WAIT, || {
        rig.rows() < before && rig.shell.tree_view().rows[0].expanded == Some(false)
    })?;
    press(&rig.row_widget(first)?, gtk::gdk::Key::Right)?;
    settle("Right expanded the folder", WAIT, || rig.rows() == before)?;
    // Enter on a session opens it
    let session = rig.node("Client emails").ok_or("no 'Client emails'")?;
    let NodeId::Session(sid) = session else {
        return Err("not a session".into());
    };
    let row = rig.row_widget(session)?;
    rig.shell.tree().focus_node(Some(session));
    press(&row, gtk::gdk::Key::Return)?;
    settle("Enter selected the session", WAIT, || {
        rig.shell.tree_view().selected == Some(sid)
    })?;
    if !rig.row_widget(session)?.has_css_class("selected") {
        return Err("the selected row has no selected class".into());
    }
    // Home and End
    press(&rig.row_widget(session)?, gtk::gdk::Key::End)?;
    let last = rig
        .shell
        .tree_view()
        .rows
        .last()
        .map(|r| r.node)
        .ok_or("no rows")?;
    settle("End: focus on the last row", WAIT, || {
        rig.row_widget(last).is_ok_and(|w| focus_is(&w))
    })?;
    // action buttons are keyboard-accessible: on the focused row they are visible and focusable
    let actions = tooltip_button(&rig.row_widget(last)?, "Delete")?;
    if !actions.is_visible() || !actions.is_focusable() {
        return Err("row actions are not visible on focus".into());
    }
    let _ = tree;
    if !ui::theme_tokens::css().contains(":focus-visible") {
        return Err("no visible focus in the CSS".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
