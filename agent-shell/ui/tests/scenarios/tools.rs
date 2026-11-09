//! Tools column and browser in real GTK under headless Wayland: visibility, tabs, width,
//! start page without a network, "Home", a profile per session, hostile address.

use controllers::{BusEvent, Tab};
use gtk4 as gtk;
use gtk4::prelude::*;
use ui::shell::ColumnId;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{Rig, find_all, open_wide as open, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_tools_visibility_and_tabs", visibility_and_tabs),
        ("ui_tools_fits_column_width", fits_column_width),
        ("ui_browser_start_page_works_offline", start_page_offline),
        ("ui_browser_home_does_not_reload_app", home_keeps_state),
        ("ui_browser_profile_per_session", profile_per_session),
        ("ui_browser_address_is_literal", address_literal),
    ]
}

/// Ids of the two top sessions (not threads) from the core.
pub(crate) fn two_sessions(ctx: &Ctx, rig: &Rig) -> Result<(i64, i64), String> {
    let list = rig
        .call(ctx, rig.http.list_sessions())
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = list
        .iter()
        .filter(|s| s["parent_id"].is_null())
        .filter_map(|s| s["id"].as_i64())
        .collect();
    match ids.as_slice() {
        [a, b, ..] => Ok((*a, *b)),
        _ => Err("fewer than two sessions in the fixtures".into()),
    }
}

pub(crate) fn open_tab(rig: &Rig, session_id: i64, tab: Tab) -> Result<(), String> {
    rig.shell
        .bus()
        .publish(BusEvent::OpenSession { session_id, tab });
    settle("tools column shown", WAIT, || {
        rig.shell.tools_view().visible && rig.shell.tools_view().session == Some(session_id)
    })
}

fn visibility_and_tabs(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    if rig.shell.column(ColumnId::Tools).is_visible() {
        return Err("without an open session the tools column must be hidden".into());
    }
    let (a, b) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Chat)?;
    settle("column visible", WAIT, || {
        rig.shell.column(ColumnId::Tools).is_visible()
    })?;
    let widgets = rig.shell.tools_widgets();
    if widgets.buttons.len() != 3 {
        return Err(format!("tabs {}, expected 3", widgets.buttons.len()));
    }
    let terminal = widgets.button(Tab::Terminal).ok_or("no terminal button")?;
    terminal.set_active(true);
    settle("terminal tab", WAIT, || {
        rig.shell.tools_view().active == Tab::Terminal
    })?;
    open_tab(&rig, b, Tab::Chat)?;
    if rig.shell.tools_view().active != Tab::Terminal || !terminal.is_active() {
        return Err("changing the session reset the tab".into());
    }
    // a click on the collapsed column icon expands it
    rig.shell
        .collapse_button(controllers::Column::Tools)
        .emit_clicked();
    settle("column collapsed", WAIT, || {
        rig.shell.tools_view().collapsed
    })?;
    if widgets.stack.is_visible() {
        return Err("the page is visible in the collapsed column".into());
    }
    widgets
        .button(Tab::SessionSettings)
        .ok_or("no settings button")?
        .set_active(true);
    settle("column expanded by selecting a tab", WAIT, || {
        !rig.shell.tools_view().collapsed
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn fits_column_width(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Chat)?;
    let widgets = rig.shell.tools_widgets();
    let mut over = Vec::new();
    for tab in [Tab::Browser, Tab::Terminal, Tab::SessionSettings] {
        widgets.button(tab).ok_or("no button")?.set_active(true);
        settle("page", WAIT, || rig.shell.tools_view().active == tab)?;
        let (min, _, _, _) = widgets.stack.measure(gtk::Orientation::Horizontal, -1);
        if min > 520 {
            over.push(format!("{tab:?}: {min} px"));
        }
    }
    if over.is_empty() {
        rig.shell.window().destroy();
        Ok(())
    } else {
        Err(format!(
            "page wider than the column (560 px): {}",
            over.join(", ")
        ))
    }
}

fn start_page_offline(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Browser)?;
    let pane = rig.shell.browser_pane();
    settle("quick links from the core", WAIT, || {
        pane.start.quick_titles().len() == 5
    })?;
    let titles = pane.start.quick_titles();
    for want in ["Google", "YouTube", "GitHub", "Wikipedia", "HackerNews"] {
        if !titles.iter().any(|t| t == want) {
            return Err(format!("no quick link {want}: {titles:?}"));
        }
    }
    if !rig.shell.browser_view().start_page {
        return Err("start page not shown".into());
    }
    // "Add" really adds a link and writes it to the core
    pane.start.fill_add("Example", "example.org");
    pane.start.add.emit_clicked();
    settle("link added", WAIT, || pane.start.quick_titles().len() == 6)?;
    settle("link written to the core", WAIT, || {
        rig.call(ctx, rig.http.quicklinks())
            .is_ok_and(|l| l.items.len() == 6 && l.items[5]["url"] == "https://example.org")
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn home_keeps_state(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Browser)?;
    let pane = rig.shell.browser_pane();
    let rows = rig.shell.tree_view().rows.len();
    let selected = rig.shell.tree_view().selected;
    pane.address.set_text("example.org");
    pane.address.emit_activate();
    settle("session page created", WAIT, || pane.has_webview(a))?;
    pane.home.emit_clicked();
    settle("start page", WAIT, || rig.shell.browser_view().start_page)?;
    if rig.shell.tree_view().rows.len() != rows
        || rig.shell.tree_view().selected != selected
        || rig.shell.tools_view().session != Some(a)
    {
        return Err(
            "'Home' changed the list or the session selection (`location.reload()` regression)"
                .into(),
        );
    }
    if !rig.shell.window().is_visible() {
        return Err("window restarted".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn profile_per_session(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, b) = two_sessions(ctx, &rig)?;
    let pane = rig.shell.browser_pane();
    open_tab(&rig, a, Tab::Browser)?;
    pane.address.set_text("one.example.com");
    pane.address.emit_activate();
    settle("first session page", WAIT, || pane.has_webview(a))?;
    open_tab(&rig, b, Tab::Browser)?;
    if !rig.shell.browser_view().start_page || pane.shows_page_of(a) {
        return Err("the second session must have its own start page".into());
    }
    pane.address.set_text("two.example.com");
    pane.address.emit_activate();
    settle("second session page", WAIT, || pane.has_webview(b))?;
    open_tab(&rig, a, Tab::Browser)?;
    settle("first page in place", WAIT, || pane.shows_page_of(a))?;
    let base = std::path::PathBuf::from(std::env::var("XDG_DATA_HOME").map_err(|e| e.to_string())?)
        .join("agent-shell/browser/sessions");
    for id in [a, b] {
        if !base.join(id.to_string()).join("data").is_dir() {
            return Err(format!(
                "no session profile directory {id} in {}",
                base.display()
            ));
        }
    }
    rig.shell.window().destroy();
    Ok(())
}

fn address_literal(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Browser)?;
    let pane = rig.shell.browser_pane();
    for evil in [
        "https://x<img src=x onerror=alert(1)>".to_string(),
        "\"><script>alert(1)</script>".to_string(),
        "<".repeat(100_000),
    ] {
        pane.address.set_text(&evil);
        pane.address.emit_activate();
        settle("handled", WAIT, || {
            gtk::glib::MainContext::default().iteration(false);
            true
        })?;
        let view = rig.shell.browser_view();
        if let Some(load) = &view.load
            && !(load.url.starts_with("https://") || load.url.starts_with("http://"))
        {
            return Err(format!("load is not http(s): {:.50}", load.url));
        }
        if view.address.chars().count() > 2049 {
            return Err("the address in the field is not bounded".into());
        }
    }
    // interface labels do not parse markup
    for l in find_all::<gtk::Label>(rig.shell.column(ColumnId::Tools).upcast_ref()) {
        if l.uses_markup() {
            return Err(format!("label {:?} parses markup", l.text()));
        }
    }
    rig.shell.window().destroy();
    Ok(())
}
