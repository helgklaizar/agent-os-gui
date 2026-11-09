//! "Sessions" column in real GTK: collapsed column, hostile names, 500 sessions.

use std::time::{Duration, Instant};

use controllers::Column;
use controllers::tree_types::NodeId;
use core_client::{Endpoint, HttpClient, Token};
use fake_agentd::{Config, FakeAgentd};
use gtk4 as gtk;
use gtk4::prelude::*;
use ui::shell::ColumnId;

use crate::ctx::{Ctx, Scenario as Run, WAIT, settle};
use crate::tree::{Rig, find_all, labels, open, open_with, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Run)> {
    vec![
        ("ui_tree_collapsed_badges", collapsed_badges),
        ("ui_tree_evil_names_literal", evil_names),
        ("ui_tree_500_sessions", five_hundred),
    ]
}

fn collapsed_badges(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    rig.shell.collapse_button(Column::Sessions).emit_clicked();
    settle("64 px column", WAIT, || {
        rig.shell.column(ColumnId::Sessions).width() == 64
    })
    .map_err(|e| {
        format!(
            "{e}; window {}x{}, sessions column {} px",
            rig.shell.window().width(),
            rig.shell.window().height(),
            rig.shell.column(ColumnId::Sessions).width()
        )
    })?;
    settle("strip with total 6", WAIT, || {
        rig.shell
            .tree_view()
            .strip
            .as_ref()
            .is_some_and(|s| s.badge.as_deref() == Some("6"))
    })?;
    let column = rig.shell.column(ColumnId::Sessions);
    let shown = labels(&column);
    for want in ["6", "3", "2", "1"] {
        if !shown.iter().any(|t| t == want) {
            return Err(format!(
                "the collapsed column has no badge '{want}': {shown:?}"
            ));
        }
    }
    let mut buttons = find_all::<gtk::Button>(column.upcast_ref());
    buttons.retain(|b| b.tooltip_text().is_some_and(|t| t.contains("unread")));
    if buttons.len() != 3 {
        return Err(format!(
            "there must be 3 session icons with unread, found {}",
            buttons.len()
        ));
    }
    // a new unread arrives as a frame and is visible without expanding
    let Some(NodeId::Session(other)) = rig.node("No folder") else {
        return Err("no 'No folder'".into());
    };
    rig.d.add_unread(other, 4).map_err(|e| e.to_string())?;
    settle("total 10", WAIT, || {
        rig.shell
            .tree_view()
            .strip
            .is_some_and(|s| s.badge.as_deref() == Some("10"))
    })?;
    // opening a session resets the counter in the core and in the view
    let Some(NodeId::Session(release)) = rig.node("Release 1.2").or(None) else {
        let target = rig
            .shell
            .tree_view()
            .strip
            .and_then(|s| s.items.first().map(|i| i.session_id));
        let id = target.ok_or("no icons")?;
        return open_and_reset(&rig, id);
    };
    open_and_reset(&rig, release)
}

fn open_and_reset(rig: &Rig, id: i64) -> Result<(), String> {
    let button = find_all::<gtk::Button>(rig.shell.column(ColumnId::Sessions).upcast_ref())
        .into_iter()
        .find(|b| b.tooltip_text().is_some_and(|t| t.contains("unread")))
        .ok_or("no icon")?;
    button.emit_clicked();
    settle("the open session's badge disappeared", WAIT, || {
        rig.shell
            .tree_view()
            .strip
            .is_some_and(|s| s.items.iter().all(|i| i.session_id != id) || s.total_unread < 10)
    })?;
    settle("unread = 0 in the DB for the open one", WAIT, || {
        rig.d
            .db()
            .reader()
            .ok()
            .and_then(|c| {
                c.query_row("SELECT sum(unread) FROM sessions", [], |x| {
                    x.get::<_, i64>(0)
                })
                .ok()
            })
            .is_some_and(|n| n < 10)
    })?;
    rig.shell.collapse_button(Column::Sessions).emit_clicked();
    settle("column expanded", WAIT, || {
        rig.shell.column(ColumnId::Sessions).width() == 236
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn evil_names(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let evil = [
        "<img src=x onerror=alert(1)>",
        "\" onmouseover=\"x",
        "<b>bold</b> &amp;",
    ];
    for name in evil {
        let made = rig.call(ctx, rig.http.create_session(name, None, None));
        made.map_err(|e| e.to_string())?;
    }
    settle("hostile names in the tree", WAIT, || {
        evil.iter().all(|n| rig.node(n).is_some())
    })?;
    let shown = labels(&rig.shell.column(ColumnId::Sessions));
    for name in evil {
        if !shown.iter().any(|t| t == name) {
            return Err(format!("name '{name}' is not shown literally: {shown:?}"));
        }
    }
    for label in find_all::<gtk::Label>(rig.shell.column(ColumnId::Sessions).upcast_ref()) {
        if label.uses_markup() {
            return Err(format!("caption '{}' uses markup", label.text()));
        }
    }
    rig.shell.window().destroy();
    Ok(())
}

fn five_hundred(ctx: &Ctx) -> Result<(), String> {
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(Config::temp().no_ticker()))
        .map_err(|e| e.to_string())?;
    let endpoint = Endpoint::Unix(d.socket_path().to_path_buf());
    let http = HttpClient::new(endpoint, Some(Token::new(d.admin_token())));
    ctx.rt.block_on(async {
        for i in 0..500 {
            http.create_session(&format!("Session {i}"), None, None)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })?;
    let started = Instant::now();
    let rig = open_with(ctx, d)?;
    settle("507 rows", Duration::from_secs(15), || rig.rows() >= 507)?;
    let loaded_in = started.elapsed();
    let adjustment = rig.shell.tree().scroller().vadjustment();
    settle("list is scrollable", WAIT, || {
        adjustment.upper() > adjustment.page_size() * 5.0
    })
    .map_err(|e| {
        format!(
            "{e}; window {}x{}, upper {}, page_size {}",
            rig.shell.window().width(),
            rig.shell.window().height(),
            adjustment.upper(),
            adjustment.page_size()
        )
    })?;
    let ctxm = gtk::glib::MainContext::default();
    let clock = rig
        .shell
        .tree()
        .scroller()
        .frame_clock()
        .ok_or("the list has no frame clock")?;
    let (mut worst, mut painted) = (Duration::ZERO, 0u32);
    let steps = 100;
    let range = adjustment.upper() - adjustment.page_size();
    for i in 0..=steps {
        let frame_before = clock.frame_counter();
        let t = Instant::now();
        adjustment.set_value(range * f64::from(i) / f64::from(steps));
        // wait for a real frame: the frame clock counter must grow, otherwise there was no redraw
        settle("frame after a scroll step", WAIT, || {
            clock.frame_counter() > frame_before
        })?;
        while ctxm.iteration(false) {}
        painted += 1;
        worst = worst.max(t.elapsed());
    }
    if painted != steps + 1 {
        return Err(format!("painted {painted} frames out of {}", steps + 1));
    }
    println!(
        "    ui_tree_500_sessions: loading {loaded_in:?}, worst scroll step with a frame {worst:?}, frames {painted}, rows {}",
        rig.rows()
    );
    if worst > Duration::from_millis(250) {
        return Err(format!(
            "scrolling 500 sessions is slow: worst step {worst:?}"
        ));
    }
    rig.shell.window().destroy();
    Ok(())
}
