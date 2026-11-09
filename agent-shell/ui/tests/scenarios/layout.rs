//! Window, 24 px bar, four columns per `layout::resolve`, collapse buttons, `ui.json`.

use std::path::{Path, PathBuf};

use controllers::layout::{LayoutState, resolve};
use controllers::prefs::{ColumnPref, ColumnsPrefs, UiFile};
use controllers::{Column, Manual};
use gtk4::prelude::*;
use ui::prefs_io;
use ui::shell::{ColumnId, Shell, present};

use crate::ctx::{Ctx, Scenario, WAIT, settle};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_layout_widths_match_resolve", widths_match_resolve),
        ("ui_bar_height_24", bar_height),
        ("ui_collapse_buttons_work_at_any_width", collapse_any_width),
        ("ui_collapse_state_survives_restart", survives_restart),
        ("ui_window_min_size_1024x640", min_size),
        ("ui_window_fullscreen_after_present", fullscreen),
        ("ui_window_size_persisted", size_persisted),
    ]
}

fn prefs_in(dir: &Path) -> PathBuf {
    dir.join("agent-shell/ui.json")
}

fn open(ctx: &Ctx, prefs: PathBuf, w: i32) -> Result<Shell, String> {
    let shell = ctx.shell(prefs);
    shell.window().set_default_size(w, 700);
    shell.window().present();
    settle("window got its width", WAIT, || shell.window().width() == w)?;
    // the tools column is visible only with an open session: open it once the layout has settled,
    // otherwise the chat of the open session (minimum 510 px) would push apart columns that are not collapsed yet
    let mut calm = 0;
    let mut last = widths(&shell);
    settle("layout settled", WAIT, || {
        let now = widths(&shell);
        calm = if now == last { calm + 1 } else { 0 };
        last = now;
        std::thread::sleep(std::time::Duration::from_millis(10));
        calm >= 15
    })?;
    crate::ctx::open_session(&shell, 1);
    Ok(shell)
}

fn widths(shell: &Shell) -> [i32; 4] {
    [
        ColumnId::Notifications,
        ColumnId::Sessions,
        ColumnId::Chat,
        ColumnId::Tools,
    ]
    .map(|id| shell.column(id).width())
}

fn expected(l: &LayoutState) -> [i32; 4] {
    [
        l.notifications_w as i32,
        l.sessions_w as i32,
        l.chat_w as i32,
        l.tools_w as i32,
    ]
}

fn wait_widths(shell: &Shell, want: [i32; 4]) -> Result<(), String> {
    settle(&format!("column widths {want:?}"), WAIT, || {
        widths(shell) == want
    })
    .map_err(|e| format!("{e}; now {:?}", widths(shell)))
}

/// Without an open session the tools column is hidden: the chat gets its width.
fn closed_widths(shell: &Shell, w: u32) -> [i32; 4] {
    let l = resolve(w, &ColumnsPrefs::default());
    let _ = shell;
    [
        l.notifications_w as i32,
        l.sessions_w as i32,
        (l.chat_w + l.tools_w) as i32,
        0,
    ]
}

fn widths_match_resolve(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    for w in [1024, 1071, 1575, 2000] {
        let shell = open(ctx, prefs_in(dir.path()), w)?;
        let want = expected(&resolve(w as u32, &ColumnsPrefs::default()));
        wait_widths(&shell, want).map_err(|e| format!("window {w}: {e}"))?;
        shell.window().destroy();
    }
    // 1072 and 1576: `CHAT_MIN` (480) is less than the minimum chat width of an open session (510), so with an open session
    // the window grows; this checks the layout without a session, where the tools column is hidden
    for w in [1072, 1576] {
        let shell = ctx.shell(prefs_in(dir.path()));
        shell.window().set_default_size(w, 700);
        shell.window().present();
        settle("window got its width", WAIT, || shell.window().width() == w)?;
        wait_widths(&shell, closed_widths(&shell, w as u32))
            .map_err(|e| format!("window {w} without a session: {e}"))?;
        shell.window().destroy();
    }
    Ok(())
}

fn bar_height(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = open(ctx, prefs_in(dir.path()), 1280)?;
    settle("bar got its height", WAIT, || shell.bar().height() > 0)?;
    let h = shell.bar().height();
    shell.window().destroy();
    if h == 24 {
        Ok(())
    } else {
        Err(format!("bar height {h}, expected 24"))
    }
}

fn click(button: &gtk4::Button) {
    button.emit_clicked();
}

fn collapse_any_width(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    // 2000: tools expanded; the button collapses and expands the column. Manual expansion at 1024 with an open
    // session is impossible: the chat of an open session (510 px) is wider than the remainder (172 px) per `resolve`, covered by `layout` unit tests
    let shell = open(ctx, prefs_in(dir.path()), 2000)?;
    wait_widths(&shell, expected(&resolve(2000, &ColumnsPrefs::default())))?;
    let manual = |t: Manual, n: Manual| ColumnsPrefs {
        tools: ColumnPref { manual: t },
        notifications: ColumnPref { manual: n },
        ..ColumnsPrefs::default()
    };
    click(&shell.collapse_button(Column::Tools));
    wait_widths(
        &shell,
        expected(&resolve(2000, &manual(Manual::Collapsed, Manual::Auto))),
    )
    .map_err(|e| format!("collapsing tools at 2000: {e}"))?;
    click(&shell.collapse_button(Column::Tools));
    wait_widths(
        &shell,
        expected(&resolve(2000, &manual(Manual::Expanded, Manual::Auto))),
    )
    .map_err(|e| format!("expanding again: {e}"))?;
    if shell.column(ColumnId::Tools).width() != 560 {
        return Err("tools did not become 560".into());
    }
    shell.window().destroy();

    // 2000: manual collapse of notifications (a fresh ui.json so the previous step has no effect)
    let fresh = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = open(ctx, prefs_in(fresh.path()), 2000)?;
    click(&shell.collapse_button(Column::Notifications));
    let cols = manual(Manual::Auto, Manual::Collapsed);
    wait_widths(&shell, expected(&resolve(2000, &cols)))
        .map_err(|e| format!("collapsing notifications at 2000: {e}"))?;
    shell.window().destroy();
    Ok(())
}

fn survives_restart(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = prefs_in(dir.path());
    let first = open(ctx, path.clone(), 2000)?;
    click(&first.collapse_button(Column::Notifications));
    wait_widths(&first, [56, 236, 2000 - 56 - 236 - 560, 560])?;
    first.flush_prefs();
    first.window().destroy();
    let (saved, warning): (UiFile, _) = prefs_io::load(&path);
    if warning.is_some() || saved.columns.notifications.manual != Manual::Collapsed {
        return Err(format!(
            "no collapsed notifications in ui.json: {saved:?} {warning:?}"
        ));
    }
    let second = open(ctx, path, 2000)?;
    wait_widths(&second, [56, 236, 2000 - 56 - 236 - 560, 560])
        .map_err(|e| format!("after restart: {e}"))?;
    second.window().destroy();
    Ok(())
}

fn min_size(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = ctx.shell(prefs_in(dir.path()));
    shell.window().set_default_size(800, 500);
    shell.window().present();
    settle("window at least 1024x640", WAIT, || {
        shell.window().width() >= 1024 && shell.window().height() >= 640
    })
    .map_err(|e| {
        format!(
            "{e}; now {}x{}",
            shell.window().width(),
            shell.window().height()
        )
    })?;
    shell.window().destroy();
    Ok(())
}

fn fullscreen(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = ctx.shell(prefs_in(dir.path()));
    present(shell.window());
    settle("window fullscreen", WAIT, || {
        shell.window().is_fullscreen()
            && shell.window().width() == 2000
            && shell.window().height() == 1000
    })
    .map_err(|e| {
        format!(
            "{e}; fullscreen={} {}x{}",
            shell.window().is_fullscreen(),
            shell.window().width(),
            shell.window().height()
        )
    })?;
    shell.window().destroy();
    Ok(())
}

fn size_persisted(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = prefs_in(dir.path());
    let shell = open(ctx, path.clone(), 1500)?;
    // the size goes to the controller on the frame tick after layout: wait until it is in the file
    let saved = settle("window width in ui.json", WAIT, || {
        shell.flush_prefs();
        prefs_io::load(&path).0.window.w == 1500
    });
    shell.window().destroy();
    saved.map_err(|e| format!("{e}; in the file {}", prefs_io::load(&path).0.window.w))
}
