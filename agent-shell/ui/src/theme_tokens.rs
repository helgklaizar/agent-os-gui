//! Colour tokens of both themes in one place, and the CSS built from them.
//!
//! Panels are opaque, no blur. Accents from the spec: `#0a84ff` (dark), `#007aff` (light).
//! State colours of the light theme are darker than the prototype: `#34c759`, `#ff9f0a`, `#ff3b30` have contrast below 3:1 on a light column.

use std::fmt::Write;

use controllers::ResolvedTheme;

/// sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `rgba(r, g, b, a)` for CSS (bar backing: semi-transparent, no blur).
    pub fn rgba(self, alpha: f64) -> String {
        format!("rgba({}, {}, {}, {alpha})", self.0, self.1, self.2)
    }

    /// `#rrggbb` in lowercase.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Parses `#rrggbb` (any case); other forms are rejected.
    pub fn parse(text: &str) -> Option<Rgb> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Relative luminance per WCAG 2.x.
    fn luminance(self) -> f64 {
        let lin = |v: u8| {
            let c = f64::from(v) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }
}

/// Contrast of two colours per WCAG 2.x (1 to 21).
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Colour set of one theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Window background under the panels.
    pub window: Rgb,
    /// Column background (opaque: no blur).
    pub panel: Rgb,
    /// Raised surface inside a column (card, button).
    pub raised: Rgb,
    pub line: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    /// Accent and focus ring.
    pub accent: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub bad: Rgb,
}

const DARK: Palette = Palette {
    window: Rgb(0x0f, 0x11, 0x16),
    panel: Rgb(0x1b, 0x1e, 0x26),
    raised: Rgb(0x27, 0x2b, 0x35),
    line: Rgb(0x35, 0x3a, 0x46),
    text: Rgb(0xf2, 0xf3, 0xf7),
    muted: Rgb(0x9a, 0x9f, 0xb0),
    accent: Rgb(0x0a, 0x84, 0xff),
    ok: Rgb(0x30, 0xd1, 0x58),
    warn: Rgb(0xff, 0xd6, 0x0a),
    bad: Rgb(0xff, 0x45, 0x3a),
};

const LIGHT: Palette = Palette {
    window: Rgb(0xe9, 0xeb, 0xf2),
    panel: Rgb(0xf7, 0xf8, 0xfb),
    raised: Rgb(0xeb, 0xed, 0xf3),
    line: Rgb(0xcf, 0xd3, 0xdd),
    text: Rgb(0x1c, 0x1c, 0x1e),
    muted: Rgb(0x5f, 0x64, 0x72),
    accent: Rgb(0x00, 0x7a, 0xff),
    ok: Rgb(0x1e, 0x7e, 0x34),
    warn: Rgb(0xa3, 0x52, 0x00),
    bad: Rgb(0xc9, 0x16, 0x2b),
};

pub fn palette(theme: ResolvedTheme) -> Palette {
    match theme {
        ResolvedTheme::Dark => DARK,
        ResolvedTheme::Light => LIGHT,
    }
}

/// Theme class name on the window.
pub fn class_name(theme: ResolvedTheme) -> &'static str {
    match theme {
        ResolvedTheme::Dark => "theme-dark",
        ResolvedTheme::Light => "theme-light",
    }
}

/// Opacity of the bar backing.
pub const BAR_PLATE_ALPHA: f64 = 0.98;

/// A "text/background" pair with a contrast threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastPair {
    pub name: &'static str,
    pub fg: Rgb,
    pub bg: Rgb,
    /// 4.5 for normal text, 3.0 for large text and icons (WCAG AA).
    pub min: f64,
}

/// Colour `fg` with opacity `alpha` over `under`.
pub fn composite(fg: Rgb, alpha: f64, under: Rgb) -> Rgb {
    let mix = |f: u8, u: u8| (f64::from(f) * alpha + f64::from(u) * (1.0 - alpha)).round() as u8;
    Rgb(mix(fg.0, under.0), mix(fg.1, under.1), mix(fg.2, under.2))
}

/// Text colour on the accent (counter badges): white if it holds 4.5:1, otherwise black.
pub fn badge_text(p: &Palette) -> Rgb {
    let white = Rgb(0xff, 0xff, 0xff);
    if contrast(p.accent, white) >= 4.5 {
        white
    } else {
        Rgb(0, 0, 0)
    }
}

/// All "text/background" pairs of a theme that are checked (WCAG AA: 4.5:1 for text, 3:1 for accent and icons).
pub fn contrast_table(theme: ResolvedTheme) -> Vec<ContrastPair> {
    let p = palette(theme);
    let pair = |name, fg, bg, min| ContrastPair { name, fg, bg, min };
    // the bar sits on the wallpaper: the worst case is the darkest and lightest wallpapers under a nearly opaque backing
    let dark = composite(p.panel, BAR_PLATE_ALPHA, Rgb(0, 0, 0));
    let light = composite(p.panel, BAR_PLATE_ALPHA, Rgb(255, 255, 255));
    vec![
        pair("text/window", p.text, p.window, 4.5),
        pair("text/panel", p.text, p.panel, 4.5),
        pair("text/raised", p.text, p.raised, 4.5),
        pair("muted/panel", p.muted, p.panel, 4.5),
        pair("muted/raised", p.muted, p.raised, 4.5),
        pair("accent/panel", p.accent, p.panel, 3.0),
        pair("accent/window", p.accent, p.window, 3.0),
        pair("ok/panel", p.ok, p.panel, 4.5),
        pair("warn/panel", p.warn, p.panel, 4.5),
        pair("bad/panel", p.bad, p.panel, 4.5),
        pair("badge-text/accent", badge_text(&p), p.accent, 4.5),
        pair("bar-text/dark-wallpaper", p.text, dark, 4.5),
        pair("bar-text/light-wallpaper", p.text, light, 4.5),
        pair("bar-muted/dark-wallpaper", p.muted, dark, 4.5),
        pair("bar-muted/light-wallpaper", p.muted, light, 4.5),
        pair("bar-ok/dark-wallpaper", p.ok, dark, 4.5),
        pair("bar-ok/light-wallpaper", p.ok, light, 4.5),
        pair("bar-warn/dark-wallpaper", p.warn, dark, 4.5),
        pair("bar-warn/light-wallpaper", p.warn, light, 4.5),
        pair("bar-bad/dark-wallpaper", p.bad, dark, 4.5),
        pair("bar-bad/light-wallpaper", p.bad, light, 4.5),
    ]
}

fn rules(theme: ResolvedTheme, out: &mut String) {
    let (t, p) = (class_name(theme), palette(theme));
    let (window, panel, raised, line) =
        (p.window.hex(), p.panel.hex(), p.raised.hex(), p.line.hex());
    let (text, muted, accent) = (p.text.hex(), p.muted.hex(), p.accent.hex());
    let (ok, warn, bad) = (p.ok.hex(), p.warn.hex(), p.bad.hex());
    // text on the accent: white if it holds 4.5:1, otherwise black (the `badge-text/accent` pair in `contrast_table`)
    let badge_text = badge_text(&p).hex();
    // bar backing: a nearly opaque panel so that AA holds on any wallpaper
    let plate = p.panel.rgba(BAR_PLATE_ALPHA);
    let _ = write!(
        out,
        "\
.{t}.shell {{ background-color: {window}; color: {text}; font-size: 13.5px; }}
.{t} .bar {{ background-color: {plate}; color: {text}; font-size: 12.5px; font-feature-settings: \"tnum\"; }}
.{t} .lvl-neutral {{ color: {text}; }}
.{t} .lvl-ok {{ color: {ok}; }}
.{t} .lvl-warn {{ color: {warn}; }}
.{t} .lvl-bad {{ color: {bad}; }}
.{t} .lvl-nodata {{ color: {muted}; }}
.{t} progressbar.quota-bar trough {{ min-height: 3px; background-color: {line}; }}
.{t} progressbar.quota-bar progress {{ min-height: 3px; background-color: {text}; }}
.{t} progressbar.lvl-ok progress {{ background-color: {ok}; }}
.{t} progressbar.lvl-warn progress {{ background-color: {warn}; }}
.{t} progressbar.lvl-bad progress {{ background-color: {bad}; }}
.{t} progressbar.lvl-nodata progress {{ background-color: {muted}; }}
.{t} .dialog-dim {{ background-color: rgba(0, 0, 0, 0.45); }}
.{t} .dialog-card {{ background-color: {panel}; color: {text}; border-radius: 12px; padding: 16px; box-shadow: 0 0 0 1px {line}; }}
.{t} .dialog-title {{ font-weight: 700; font-size: 15px; margin-bottom: 8px; }}
.{t} .dialog-key {{ color: {muted}; }}
.{t} .menu-card {{ background-color: {panel}; color: {text}; box-shadow: 0 0 0 1px {line}; }}
.{t} .column {{ background-color: {panel}; color: {text}; }}
.{t} .column-start {{ box-shadow: inset -1px 0 {line}; }}
.{t} .column-end {{ box-shadow: inset 1px 0 {line}; }}
.{t} .column-header {{ box-shadow: inset 0 -1px {line}; }}
.{t} .column-footer {{ box-shadow: inset 0 1px {line}; }}
.{t} .column-title {{ font-weight: 600; }}
.{t} .muted, .{t} .empty-hint {{ color: {muted}; }}
.{t} button.shell-btn {{ background: {raised}; color: {text}; border-radius: 9px; box-shadow: none; }}
.{t} button.shell-btn:hover {{ background: {line}; }}
.{t} button.shell-btn:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.{t} .logo {{ background: {accent}; border-radius: 50%; min-width: 12px; min-height: 12px; }}
.{t} button.bar-btn {{ background: transparent; border-radius: 5px; }}
.{t} button.bar-btn:hover {{ background: {raised}; }}
.{t} .tree-row {{ color: {text}; }}
.{t} .tree-row:hover {{ background: {raised}; }}
.{t} .tree-row.selected {{ background: {raised}; box-shadow: inset 2px 0 {accent}; }}
.{t} .tree-row:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.{t} .tree-badge {{ background: {accent}; color: {badge_text}; border-radius: 8px; padding: 0 5px; font-size: 11px; font-weight: 700; }}
.{t} .tree-action:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.{t} .chat-bubble {{ background: {raised}; color: {text}; border-radius: 12px; padding: 8px 12px; }}
.{t} .chat-bubble.mine {{ background: {accent}; color: {badge_text}; }}
.{t} .chat-bubble.mine .chat-meta, .{t} .chat-bubble.mine .chat-note {{ color: {badge_text}; }}
.{t} .chat-bubble.pending {{ opacity: 0.7; }}
.{t} .chat-meta, .{t} .chat-note, .{t} .chat-typing {{ color: {muted}; font-size: 11.5px; }}
.{t} .chat-code {{ font-family: monospace; background: {panel}; color: {text}; border-radius: 6px; padding: 6px; }}
.{t} .chat-h {{ font-weight: 700; }}
.{t} .chat-card {{ background: {raised}; color: {text}; border-radius: 10px; padding: 8px 12px; box-shadow: 0 0 0 1px {line}; }}
.{t} .chat-card-title {{ font-weight: 700; }}
.{t} .chat-observer, .{t} .chat-error {{ box-shadow: 0 0 0 2px {bad}; }}
.{t} .chat-card-state {{ color: {muted}; }}
.{t} .chat-chip {{ border-radius: 14px; padding: 2px 10px; }}
.{t} .chat-input {{ background: {raised}; color: {text}; border-radius: 8px; padding: 6px; }}
.{t} .chat-input:focus-within {{ outline: 2px solid {accent}; }}
.{t} button.card-btn-ok {{ background: {accent}; color: {badge_text}; }}
.{t} button.card-btn-deny {{ background: {raised}; color: {text}; box-shadow: 0 0 0 1px {line}; }}
.{t} .notif-card {{ background: {raised}; color: {text}; border-radius: 8px; }}
.{t} .notif-ai {{ box-shadow: inset 3px 0 {accent}; }}
.{t} .notif-task {{ box-shadow: inset 3px 0 {warn}; }}
.{t} .notif-msg {{ box-shadow: inset 3px 0 {ok}; }}
.{t} .notif-time, .{t} .notif-session {{ color: {muted}; font-size: 11.5px; }}
.{t} .notif-new {{ color: {text}; font-weight: 700; font-size: 11px; }}
.{t} .notif-sub {{ font-weight: 600; }}
.{t} .notif-chip.active {{ background: {line}; box-shadow: inset 0 -2px {accent}; }}
.{t} .notif-card button:focus-visible, .{t} .notif-toast:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.{t} .notif-toast {{ background: {panel}; color: {text}; border-radius: 10px; box-shadow: 0 0 0 1px {line}; }}
.{t} .dialog-error {{ color: {bad}; }}
.{t} .dialog-list {{ box-shadow: 0 0 0 1px {line}; border-radius: 6px; }}
.{t} .set-item {{ background: {raised}; color: {text}; border-radius: 8px; box-shadow: 0 0 0 1px {line}; }}
.{t} .set-item.selected {{ box-shadow: inset 3px 0 {accent}, 0 0 0 1px {line}; }}
.{t} button.set-nav-btn {{ background: transparent; color: {text}; border-radius: 6px; box-shadow: none; }}
.{t} button.set-nav-btn:hover {{ background: {raised}; }}
.{t} button.set-nav-btn.selected {{ background: {raised}; box-shadow: inset 2px 0 {accent}; }}
.{t} button.set-nav-btn:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.{t} .dirty {{ box-shadow: inset 2px 0 {accent}; }}
"
    );
}

/// CSS for both themes: rules under `.theme-dark` and `.theme-light`, everything from tokens.
pub fn css() -> String {
    let mut out = String::from(
        "button.shell-btn { min-height: 0; min-width: 0; padding: 6px; }\n\
         button.bar-btn { min-height: 0; padding: 0 6px; }\n\
         button.quota-btn { padding: 0 2px; }\n\
         progressbar.quota-bar trough { min-width: 16px; min-height: 3px; }\n\
         progressbar.quota-bar progress { min-height: 3px; }\n\
         .menu-card { padding: 4px; border-radius: 8px; }\n\
         .menu-card button.menu-item { padding: 4px 12px; min-width: 150px; }\n\
         .bar label.metric, .bar label.quota-pct, .bar label.quota-rate { min-height: 0; }\n\
         .dialog-card button.shell-btn { padding: 6px 14px; }\n\
         .column-title { margin: 0 12px; }\n\
         .tree-row { padding: 3px 6px; min-height: 24px; border-radius: 6px; }\n\
         .tree-buttons { padding: 6px; }\n\
         .tree-strip { padding: 6px 0; }\n\
         .notif-filters { padding: 6px 8px 0 8px; }\n\
         .notif-card { padding: 8px 10px; margin: 4px 8px; }\n\
         .notif-toast { padding: 10px 14px; margin: 12px; }\n\
         .notif-chip { padding: 4px 8px; }\n\
         .tree-action { min-height: 0; min-width: 0; padding: 2px; }\n\
         .dialog-card entry { min-width: 280px; }\n\
         .chat-items, .chat-chips, .chat-compose, .chat-persona-list { padding: 8px; }\n\
         .chat-bubble { padding: 8px 12px; }\n\
         button.set-nav-btn { min-height: 0; padding: 6px 10px; }\n\
         .set-item { padding: 8px 10px; margin: 2px 0; }\n\
         .dirty { padding-left: 6px; }\n",
    );
    rules(ResolvedTheme::Dark, &mut out);
    rules(ResolvedTheme::Light, &mut out);
    out
}
