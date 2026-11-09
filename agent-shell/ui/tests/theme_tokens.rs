//! Colour tokens: AA contrast in both themes, accents from the spec, CSS without blur.

use controllers::ResolvedTheme;
use ui::theme_tokens::{Palette, Rgb, contrast, css, palette};

const BOTH: [ResolvedTheme; 2] = [ResolvedTheme::Dark, ResolvedTheme::Light];

fn name(t: ResolvedTheme) -> &'static str {
    match t {
        ResolvedTheme::Dark => "dark",
        ResolvedTheme::Light => "light",
    }
}

fn check(t: ResolvedTheme, what: &str, fg: Rgb, bg: Rgb, min: f64) {
    let c = contrast(fg, bg);
    assert!(
        c >= min,
        "{}: {what} {} on {} gives {c:.2}:1, need at least {min}:1",
        name(t),
        fg.hex(),
        bg.hex()
    );
}

#[test]
fn theme_tokens_contrast_aa() {
    for t in BOTH {
        let p = palette(t);
        // main text is 4.5:1 on every surface it sits on
        check(
            t,
            "main text on the window background",
            p.text,
            p.window,
            4.5,
        );
        check(t, "main text on the column", p.text, p.panel, 4.5);
        check(t, "main text on the raised surface", p.text, p.raised, 4.5);
        // muted text is text too
        check(t, "muted text on the column", p.muted, p.panel, 4.5);
        check(
            t,
            "muted text on the raised surface",
            p.muted,
            p.raised,
            4.5,
        );
        // controls and states are 3:1
        check(
            t,
            "accent and focus ring on the column",
            p.accent,
            p.panel,
            3.0,
        );
        check(
            t,
            "accent and focus ring on the window background",
            p.accent,
            p.window,
            3.0,
        );
        for (what, c) in [("ok", p.ok), ("warn", p.warn), ("bad", p.bad)] {
            check(
                t,
                &format!("state colour {what} on the column"),
                c,
                p.panel,
                3.0,
            );
        }
    }
}

#[test]
fn theme_tokens_accents_from_tz() {
    // accent #0a84ff in the dark theme and #007aff in the light one
    assert_eq!(palette(ResolvedTheme::Dark).accent.hex(), "#0a84ff");
    assert_eq!(palette(ResolvedTheme::Light).accent.hex(), "#007aff");
}

#[test]
fn theme_tokens_themes_differ() {
    let (d, l): (Palette, Palette) = (palette(ResolvedTheme::Dark), palette(ResolvedTheme::Light));
    assert_ne!(d.window, l.window);
    assert_ne!(d.text, l.text);
    assert!(
        contrast(d.window, l.window) > 8.0,
        "theme backgrounds must be opposite in luminance"
    );
}

#[test]
fn contrast_matches_wcag_reference_values() {
    let (white, black) = (Rgb(255, 255, 255), Rgb(0, 0, 0));
    assert!((contrast(white, black) - 21.0).abs() < 1e-9);
    assert!((contrast(white, white) - 1.0).abs() < 1e-9);
    // WCAG reference: #777777 on white is 4.48:1 (below the AA threshold), #767676 is 4.54:1 (above)
    assert!((contrast(Rgb(0x77, 0x77, 0x77), white) - 4.48).abs() < 0.01);
    assert!((contrast(Rgb(0x76, 0x76, 0x76), white) - 4.54).abs() < 0.01);
    assert_eq!(contrast(black, white), contrast(white, black));
}

#[test]
fn rgb_hex_roundtrip_and_rejects_garbage() {
    assert_eq!(Rgb(10, 132, 255).hex(), "#0a84ff");
    assert_eq!(Rgb::parse("#0a84ff"), Some(Rgb(10, 132, 255)));
    assert_eq!(Rgb::parse("#0A84FF"), Some(Rgb(10, 132, 255)));
    for bad in ["", "0a84ff", "#0a84f", "#0a84ffaa", "#gg0000", "rgb(1,2,3)"] {
        assert_eq!(Rgb::parse(bad), None, "{bad:?} is not a colour");
    }
}

#[test]
fn theme_css_covers_both_themes_and_focus() {
    let css = css();
    assert!(css.contains(".theme-dark"), "no dark theme rules");
    assert!(css.contains(".theme-light"), "no light theme rules");
    assert!(css.contains(":focus-visible"), "no visible focus");
    for t in BOTH {
        let p = palette(t);
        assert!(
            css.contains(&p.panel.hex()) && css.contains(&p.text.hex()),
            "{}: no palette tokens in the CSS",
            name(t)
        );
    }
}

#[test]
fn theme_css_has_no_blur() {
    // no blur; an empty CSS does not prove this condition
    let css = css();
    assert!(!css.is_empty());
    for banned in ["blur", "backdrop-filter", "backdrop"] {
        assert!(!css.contains(banned), "the CSS contains {banned}");
    }
}
