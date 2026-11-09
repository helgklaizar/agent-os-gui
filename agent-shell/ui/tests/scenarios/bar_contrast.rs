//! `ui_bar_contrast`: bar legibility on dark, light and mixed wallpapers in both themes.
//!
//! The bar is drawn in real GTK over a "wallpaper" (a CSS background under it), then again without content: the backing over the wallpaper remains.
//! Contrast is computed from the worst backing pixel against each theme text colour (main, ok, warn, bad, muted).
//! If `S2_SHOTS_DIR` is set, bar frames with text are saved there as PNG.

use std::path::PathBuf;
use std::time::Instant;

use controllers::ResolvedTheme;
use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::timezone::Zone;
use gtk4 as gtk;
use gtk4::prelude::*;
use serde_json::json;
use ui::shell_view::Bar;
use ui::theme_tokens::{Rgb, class_name, contrast, css, palette};

use crate::ctx::{Ctx, WAIT, settle};

const AA: f64 = 4.5;
const WIDTH: i32 = 1024;
const MOTLEY: &str = "repeating-linear-gradient(to right, #ffff00 0px, #ffff00 8px, #ff00ff 8px, #ff00ff 16px, #00ffff 16px, #00ffff 24px, #000000 24px, #000000 32px, #ffffff 32px, #ffffff 40px)";

fn wallpapers() -> [(&'static str, String); 3] {
    [
        ("dark", "background-color: #101018;".to_string()),
        ("light", "background-color: #f4f4f4;".to_string()),
        ("motley", format!("background-image: {MOTLEY};")),
    ]
}

/// A bar with all state colours: neutral, green, yellow, red and grey "no data".
fn filled_view() -> controllers::bar_cluster::BarClusterView {
    let mut c = BarCluster::new(Zone::fixed(7200));
    let now = Instant::now();
    let unix = 1_793_628_309;
    c.on_frame(
        "bar.sys",
        FrameKind::Snapshot,
        &json!({"cpu": 14, "gpu": 55, "ram": 90, "disk": 38, "temp": 62}),
        unix,
        now,
    );
    let item = |id: i64, slug: &str, used: f64, rate: f64| json!({ "account_id": id, "slug": slug, "provider": "anthropic", "label": slug, "kind": "api_key", "window_h": 5.0, "used": used, "rate": rate, "reset_at": unix + 5800, "usage": null, "limits": null });
    let local = json!({ "account_id": 3, "slug": "ol1", "provider": "ollama", "label": "ol1", "kind": "local", "window_h": 0.0, "used": null, "rate": null, "reset_at": null, "load": 0.9, "usage": null, "limits": null });
    c.on_frame("bar.quota", FrameKind::Snapshot, &json!({ "items": [item(1, "a", 0.18, 0.4), item(2, "b", 0.62, 1.4), item(4, "c", 0.95, 2.0), local] }), unix, now);
    c.on_frame(
        "bar.weather",
        FrameKind::Snapshot,
        &json!({"temp_c": null, "icon": null, "text": null, "place": null, "updated_at": null}),
        unix,
        now,
    );
    c.tick(unix, now);
    c.view()
}

fn settle_frames() {
    for _ in 0..10 {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

fn render(
    win: &gtk::Window,
    root: &gtk::Widget,
    size: (i32, i32),
) -> Result<gtk::gdk::Texture, String> {
    let renderer = win
        .native()
        .and_then(|n| n.renderer())
        .ok_or("the window has no renderer")?;
    let paintable = gtk::WidgetPaintable::new(Some(root));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(size.0), f64::from(size.1));
    let node = snapshot.to_node().ok_or("empty render node")?;
    let area = gtk::graphene::Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32);
    Ok(renderer.render_texture(&node, Some(&area)))
}

fn pixels(tex: &gtk::gdk::Texture) -> Vec<Rgb> {
    let (w, h) = (tex.width() as usize, tex.height() as usize);
    let stride = w * 4;
    let mut data = vec![0u8; stride * h];
    tex.download(&mut data, stride);
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|p| Rgb(p[2], p[1], p[0]))
        .collect()
}

fn hide_content(bar: &Bar) {
    let mut c = bar.root.first_child();
    while let Some(w) = c {
        w.set_opacity(0.0);
        c = w.next_sibling();
    }
}

pub fn run(ctx: &Ctx) -> Result<(), String> {
    let _ = ctx;
    let display = gtk::gdk::Display::default().ok_or("no display")?;
    let theme_css = gtk::CssProvider::new();
    theme_css.load_from_string(&css());
    gtk::style_context_add_provider_for_display(
        &display,
        &theme_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let walls = gtk::CssProvider::new();
    let rules: String = wallpapers()
        .iter()
        .map(|(n, c)| format!(".wp-{n} {{ {c} }}\n"))
        .collect();
    walls.load_from_string(&rules);
    gtk::style_context_add_provider_for_display(
        &display,
        &walls,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let shots = std::env::var("S2_SHOTS_DIR").ok().map(PathBuf::from);
    let view = filled_view();
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for theme in [ResolvedTheme::Dark, ResolvedTheme::Light] {
        let pal = palette(theme);
        let colors = [
            ("text", pal.text),
            ("ok", pal.ok),
            ("warn", pal.warn),
            ("bad", pal.bad),
            ("muted", pal.muted),
        ];
        for (wall, _) in wallpapers() {
            let win = gtk::Window::new();
            win.set_default_size(WIDTH, 24);
            let wp = gtk::Box::new(gtk::Orientation::Vertical, 0);
            wp.add_css_class(class_name(theme));
            wp.add_css_class(&format!("wp-{wall}"));
            let bar = Bar::new();
            bar.widgets.apply(&view);
            wp.append(&bar.root);
            win.set_child(Some(&wp));
            win.present();
            settle("bar drawn", WAIT, || {
                wp.width() >= WIDTH && wp.height() >= 24
            })
            .map_err(|e| {
                format!(
                    "{e}: bar {}x{}, window {}x{}",
                    wp.width(),
                    wp.height(),
                    win.width(),
                    win.height()
                )
            })?;
            settle_frames();
            let size = (wp.width().min(WIDTH), 24);
            let with_text = render(&win, wp.upcast_ref(), size)?;
            if let Some(dir) = &shots {
                let name = format!("S2-bar-{}-{wall}.png", class_name(theme));
                with_text
                    .save_to_png(dir.join(name))
                    .map_err(|e| e.to_string())?;
            }
            hide_content(&bar);
            settle_frames();
            let plate = pixels(&render(&win, wp.upcast_ref(), size)?);
            let (mut worst, mut worst_name) = (f64::MAX, "");
            for p in &plate {
                for (name, color) in colors {
                    let c = contrast(color, *p);
                    if c < worst {
                        (worst, worst_name) = (c, name);
                    }
                }
            }
            report.push(format!(
                "{} / {wall}: worst contrast {worst:.2} ({worst_name})",
                class_name(theme)
            ));
            if worst < AA {
                failures.push(format!(
                    "{} / {wall}: {worst:.2} < {AA} ({worst_name})",
                    class_name(theme)
                ));
            }
            win.destroy();
        }
    }
    println!("    {}", report.join("\n    "));
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
