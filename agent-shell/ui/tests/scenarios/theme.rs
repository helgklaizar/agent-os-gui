//! Theme switching without restart, tokens on widgets, write to the core, the `ui.json` cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use controllers::prefs::UiFile;
use controllers::{ResolvedTheme, Theme};
use core_client::api;
use core_client::blocking::BlockingClient;
use core_client::{Endpoint, HttpClient, Token};
use fake_agentd::{Config, FakeAgentd};
use gtk4::prelude::*;
use libadwaita as adw;
use ui::prefs_io;
use ui::shell::{ColumnId, Shell, ShellOptions};
use ui::theme_tokens::{Rgb, palette};

use crate::ctx::{Ctx, Scenario, WAIT, settle};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_theme_switch_without_restart", switch_without_restart),
        ("ui_theme_tokens_applied_to_widgets", tokens_applied),
        ("ui_theme_cache_loaded_at_start", cache_loaded),
        ("ui_theme_choice_reaches_core", choice_reaches_core),
        ("ui_theme_core_wins_after_start", core_wins),
        ("ui_theme_offline_uses_cache", offline_uses_cache),
        ("ui_focus_reaches_every_button", focus_reaches_every_button),
    ]
}

fn has(shell: &Shell, class: &str) -> bool {
    shell.window().has_css_class(class)
}

fn prefs(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("agent-shell/ui.json")
}

fn write_cache(path: &Path, theme: Theme) -> Result<(), String> {
    let file = UiFile {
        theme,
        ..UiFile::default()
    };
    prefs_io::save(path, &file).map_err(|e| e.to_string())
}

fn switch_without_restart(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = ctx.shell(prefs(&dir));
    shell.window().present();
    let style = adw::StyleManager::default();
    let button = shell.theme_button();

    button.emit_clicked();
    settle("dark theme", WAIT, || has(&shell, "theme-dark"))?;
    if has(&shell, "theme-light") || !style.is_dark() || shell.selected_theme() != Theme::Dark {
        return Err("after the first press only the dark theme must be set".into());
    }
    button.emit_clicked();
    settle("light theme", WAIT, || has(&shell, "theme-light"))?;
    if has(&shell, "theme-dark") || style.is_dark() || shell.selected_theme() != Theme::Light {
        return Err("after the second press only the light theme must be set".into());
    }
    button.emit_clicked();
    settle("auto", WAIT, || shell.selected_theme() == Theme::Auto)?;
    if has(&shell, "theme-dark") == has(&shell, "theme-light") {
        return Err("in auto exactly one of the theme classes must be set".into());
    }
    shell.window().destroy();
    Ok(())
}

fn rgb_of(c: gtk4::gdk::RGBA) -> Rgb {
    let q = |v: f32| (v * 255.0).round() as u8;
    Rgb(q(c.red()), q(c.green()), q(c.blue()))
}

fn tokens_applied(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (theme, resolved) in [
        (Theme::Dark, ResolvedTheme::Dark),
        (Theme::Light, ResolvedTheme::Light),
    ] {
        write_cache(&prefs(&dir), theme)?;
        let shell = ctx.shell(prefs(&dir));
        shell.window().present();
        settle("theme from the cache applied", WAIT, || {
            shell.selected_theme() == theme
        })?;
        let want = palette(resolved).text;
        for id in [
            ColumnId::Notifications,
            ColumnId::Sessions,
            ColumnId::Chat,
            ColumnId::Tools,
        ] {
            settle("column text colour", WAIT, || {
                rgb_of(shell.column(id).color()) == want
            })
            .map_err(|e| {
                format!(
                    "{theme:?} {id:?}: {e}; now {}",
                    rgb_of(shell.column(id).color()).hex()
                )
            })?;
        }
        shell.window().destroy();
    }
    Ok(())
}

fn cache_loaded(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    write_cache(&prefs(&dir), Theme::Light)?;
    let shell = ctx.shell(prefs(&dir));
    shell.window().present();
    settle("light from the cache", WAIT, || {
        has(&shell, "theme-light") && !has(&shell, "theme-dark")
    })?;
    if shell.selected_theme() != Theme::Light {
        return Err(format!("selected {:?}", shell.selected_theme()));
    }
    shell.window().destroy();
    Ok(())
}

struct Core {
    d: FakeAgentd,
    http: HttpClient,
}

impl Core {
    fn start(ctx: &Ctx) -> Result<Core, String> {
        let d = ctx
            .rt
            .block_on(FakeAgentd::start(Config::temp().no_ticker()))
            .map_err(|e| e.to_string())?;
        let http = HttpClient::new(
            Endpoint::Unix(d.socket_path().to_path_buf()),
            Some(Token::new(d.admin_token())),
        );
        Ok(Core { d, http })
    }

    fn blocking(&self) -> Result<Arc<BlockingClient>, String> {
        BlockingClient::new(
            Endpoint::Unix(self.d.socket_path().to_path_buf()),
            Token::new(self.d.admin_token()),
        )
        .map(Arc::new)
        .map_err(|e| e.to_string())
    }

    fn theme(&self, ctx: &Ctx) -> Result<api::ThemePage, String> {
        ctx.rt
            .block_on(self.http.theme())
            .map_err(|e| e.to_string())
    }
}

fn with_core(ctx: &Ctx, path: PathBuf, core: Arc<BlockingClient>) -> Shell {
    Shell::new(
        &ctx.app,
        ShellOptions {
            prefs_path: path,
            core: Some(core),
            bar: ui::bar_rig::BarOptions::default(),
        },
    )
}

fn choice_reaches_core(ctx: &Ctx) -> Result<(), String> {
    let core = Core::start(ctx)?;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = with_core(ctx, prefs(&dir), core.blocking()?);
    shell.window().present();
    shell.theme_button().emit_clicked(); // Auto → Dark
    settle("core received the theme", WAIT, || {
        core.theme(ctx)
            .is_ok_and(|p| p.theme == api::Theme::Dark && p.rev == 2)
    })
    .map_err(|e| format!("{e}; in the core {:?}", core.theme(ctx)))?;
    shell.window().destroy();
    Ok(())
}

fn core_wins(ctx: &Ctx) -> Result<(), String> {
    let core = Core::start(ctx)?;
    ctx.rt
        .block_on(core.http.set_theme(1, api::Theme::Light))
        .map_err(|e| e.to_string())?;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    write_cache(&prefs(&dir), Theme::Dark)?;
    let shell = with_core(ctx, prefs(&dir), core.blocking()?);
    shell.window().present();
    settle("the core value won over the cache", WAIT, || {
        shell.selected_theme() == Theme::Light && has(&shell, "theme-light")
    })
    .map_err(|e| format!("{e}; selected {:?}", shell.selected_theme()))?;
    shell.flush_prefs();
    let (saved, _) = prefs_io::load(&prefs(&dir));
    shell.window().destroy();
    if saved.theme == Theme::Light {
        Ok(())
    } else {
        Err(format!(
            "the cache in ui.json was not overwritten: {:?}",
            saved.theme
        ))
    }
}

fn offline_uses_cache(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    write_cache(&prefs(&dir), Theme::Dark)?;
    let dead = BlockingClient::new(
        Endpoint::Unix(dir.path().join("no-such.sock")),
        Token::new("<token>"),
    )
    .map(Arc::new)
    .map_err(|e| e.to_string())?;
    let shell = with_core(ctx, prefs(&dir), dead);
    shell.window().present();
    settle("dark from the cache without a connection", WAIT, || {
        has(&shell, "theme-dark")
    })?;
    // no connection: the user's choice is applied locally, the window does not crash or hang
    shell.theme_button().emit_clicked(); // Dark → Light
    settle("light without a connection", WAIT, || {
        has(&shell, "theme-light")
    })?;
    let pause = std::time::Instant::now();
    settle(
        "stay on the selected one",
        Duration::from_millis(800),
        || false,
    )
    .ok();
    if !has(&shell, "theme-light") || pause.elapsed() < Duration::from_millis(700) {
        return Err("the theme came back or the loop did not run".into());
    }
    shell.window().destroy();
    Ok(())
}

fn buttons(root: &gtk4::Widget, out: &mut Vec<gtk4::Button>) {
    if let Some(b) = root.downcast_ref::<gtk4::Button>() {
        out.push(b.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        buttons(&c, out);
        child = c.next_sibling();
    }
}

fn focus_reaches_every_button(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let shell = ctx.shell(prefs(&dir));
    shell.window().present();
    settle("window shown", WAIT, || shell.window().width() > 0)?;
    // tools column buttons appear with an open session; unavailable buttons do not take focus
    crate::ctx::open_session(&shell, 1);
    settle("tools column shown", WAIT, || {
        shell.column(ui::shell::ColumnId::Tools).is_mapped()
    })?;
    let mut found = Vec::new();
    buttons(shell.window().upcast_ref(), &mut found);
    let all: Vec<gtk4::Button> = found
        .into_iter()
        .filter(|b| b.is_mapped() && b.is_sensitive())
        .collect();
    if all.len() < 4 {
        return Err(format!(
            "the skeleton has {} buttons, expected at least 4 (3 collapse buttons and the theme one)",
            all.len()
        ));
    }
    shell.window().set_focus_visible(true);
    for b in &all {
        if !b.is_focusable() || !b.grab_focus() {
            return Err(format!("button {:?} does not take focus", b.tooltip_text()));
        }
        settle("focus visible", WAIT, || {
            b.is_focus() && b.state_flags().contains(gtk4::StateFlags::FOCUS_VISIBLE)
        })
        .map_err(|e| format!("{e}: {:?}", b.tooltip_text()))?;
    }
    shell.window().destroy();
    Ok(())
}
