//! Chat accessibility in the AT-SPI tree of a real process and the contrast of bubbles and cards in both themes (WCAG AA).
//! Accessibility: the outer scenario restarts the runner under `dbus-run-session` with `GTK_A11Y=atspi` (like `ui_tree_selected_state`).

use std::process::Command;

use controllers::ResolvedTheme;
use controllers::i18n::tr;
use gtk4::prelude::*;
use ui::theme_tokens::{Rgb, contrast, palette};

use crate::chat::open_chat;
use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree_selected::dump;

const CHILD: &str = "ui_chat_accessibility_inner";
const FLAG: &str = "S4_A11Y_CHILD";

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    let mut list: Vec<(&'static str, Scenario)> = vec![
        ("ui_chat_accessibility", outer),
        ("ui_chat_themes_contrast", themes_contrast),
    ];
    if std::env::var_os(FLAG).is_some() {
        list.push((CHILD, inner));
    }
    list
}

fn outer(ctx: &Ctx) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("dbus-run-session");
    cmd.args(["--"]).arg(exe).arg(CHILD);
    ctx.weston.apply_env(&mut cmd);
    cmd.env("GTK_A11Y", "atspi").env(FLAG, "1");
    let out = cmd
        .output()
        .map_err(|e| format!("dbus-run-session did not start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines().filter(|l| l.starts_with("    ")) {
        println!("{line}");
    }
    if out.status.success() && text.contains(&format!("test {CHILD} ... ok")) {
        Ok(())
    } else {
        Err(format!(
            "inner scenario: code {:?}\n{text}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn inner(ctx: &Ctx) -> Result<(), String> {
    let rig = open_chat(ctx)?;
    let w = rig.shell.chat_widgets();
    let send = |text: &str| {
        w.input.buffer().set_text(text);
        w.send_btn.emit_clicked();
    };
    send("/ask send a@example.org");
    send("/danger rm -rf /");
    settle("both cards", WAIT, || {
        rig.shell.chat_view().items.len() >= 4
    })?;
    let nodes = dump()?;
    let named = |frag: &str| nodes.iter().filter(|n| n.name.contains(frag)).count();
    let mut missing = Vec::new();
    let ask = "Permission request";
    for (what, frag) in [
        ("ask card", ask),
        ("observer card", &tr("chat-observer-dangerous-cmd")),
        ("'Send' button", &tr("chat-send")),
        ("input field", &tr("chat-input-name")),
        ("persona chip", &tr("chat-chip-persona-name")),
        ("folder chip", &tr("chat-chip-path-name")),
        ("'Allow' button", &tr("chat-btn-allow")),
        ("'Attach' button", &tr("chat-attach")),
    ] {
        if named(frag) == 0 {
            missing.push(what);
        }
    }
    for n in nodes
        .iter()
        .filter(|n| n.name.contains(&tr("chat-observer-dangerous-cmd")) || n.name.contains(ask))
    {
        println!("    card: role {:?}, name {:?}", n.role, n.name);
    }
    for n in nodes.iter().filter(|n| n.role.contains("button")) {
        println!("    button: role {:?}, name {:?}", n.role, n.name);
    }
    let alert = nodes
        .iter()
        .any(|n| n.role == "alert" && n.name.contains(&tr("chat-observer-dangerous-cmd")));
    if !missing.is_empty() || !alert {
        return Err(format!(
            "not in the accessibility tree: {missing:?}; observer alert: {alert}"
        ));
    }
    rig.shell.window().destroy();
    Ok(())
}

fn themes_contrast(_: &Ctx) -> Result<(), String> {
    let mut bad = Vec::new();
    for theme in [ResolvedTheme::Dark, ResolvedTheme::Light] {
        let p = palette(theme);
        let badge_text = if contrast(p.accent, Rgb(0xff, 0xff, 0xff)) >= 4.5 {
            Rgb(0xff, 0xff, 0xff)
        } else {
            Rgb(0, 0, 0)
        };
        for (what, fg, bg) in [
            ("my bubble", badge_text, p.accent),
            ("agent bubble", p.text, p.raised),
            ("card", p.text, p.raised),
            ("agent bubble caption", p.muted, p.raised),
            ("field caption and indicator", p.muted, p.panel),
            ("code", p.text, p.panel),
            ("input field", p.text, p.raised),
        ] {
            let c = contrast(fg, bg);
            println!("    {theme:?} {what}: {c:.2}");
            if c < 4.5 {
                bad.push(format!("{theme:?} {what}: {c:.2}"));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("contrast below AA: {bad:?}"))
    }
}
