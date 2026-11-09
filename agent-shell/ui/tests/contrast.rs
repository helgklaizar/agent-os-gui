//! Contrast: a table of "text/background" pairs from the tokens of both themes, the bar on dark and light wallpapers.
//! One table serves the tests and the description of thresholds: a pair that is not in the table is not checked.

use controllers::ResolvedTheme;
use ui::theme_tokens::{BAR_PLATE_ALPHA, Rgb, composite, contrast, contrast_table, css, palette};

const BOTH: [ResolvedTheme; 2] = [ResolvedTheme::Dark, ResolvedTheme::Light];

#[test]
fn contrast_table_all_pairs_meet_aa() {
    for t in BOTH {
        let table = contrast_table(t);
        assert!(
            table.len() >= 20,
            "{t:?}: the table has {} pairs",
            table.len()
        );
        for pair in &table {
            let c = contrast(pair.fg, pair.bg);
            assert!(
                c >= pair.min,
                "{t:?}: {} — {} on {} gives {c:.2}:1, need {}:1",
                pair.name,
                pair.fg.hex(),
                pair.bg.hex(),
                pair.min
            );
        }
    }
}

#[test]
fn contrast_table_thresholds_are_wcag_only() {
    for t in BOTH {
        assert!(!contrast_table(t).is_empty(), "{t:?}: the table is empty");
        for pair in contrast_table(t) {
            assert!(
                pair.min == 4.5 || pair.min == 3.0,
                "{}: threshold {} is not from WCAG AA",
                pair.name,
                pair.min
            );
        }
    }
}

#[test]
fn contrast_table_text_pairs_use_text_threshold() {
    // anything containing the word text/muted/badge or a state colour as bar text holds 4.5; 3.0 is only for the accent and focus ring
    for t in BOTH {
        for pair in contrast_table(t) {
            let large_ok = pair.name.starts_with("accent/");
            assert!(
                pair.min == 4.5 || large_ok,
                "{}: the 3:1 threshold is allowed only for the accent and icons",
                pair.name
            );
        }
    }
}

#[test]
fn contrast_table_covers_required_surfaces() {
    let names: Vec<&str> = contrast_table(ResolvedTheme::Dark)
        .iter()
        .map(|p| p.name)
        .collect();
    for need in [
        "text/window",
        "text/panel",
        "text/raised",
        "muted/panel",
        "muted/raised",
        "accent/panel",
        "accent/window",
        "ok/panel",
        "warn/panel",
        "bad/panel",
        "badge-text/accent",
        "bar-text/dark-wallpaper",
        "bar-text/light-wallpaper",
        "bar-muted/dark-wallpaper",
        "bar-muted/light-wallpaper",
        "bar-ok/dark-wallpaper",
        "bar-warn/light-wallpaper",
        "bar-bad/light-wallpaper",
    ] {
        assert!(names.contains(&need), "the table has no pair {need}");
    }
    let light: Vec<&str> = contrast_table(ResolvedTheme::Light)
        .iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, light, "the pair sets of both themes match");
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "pair names are not repeated");
}

#[test]
fn contrast_table_uses_palette_tokens_not_copies() {
    for t in BOTH {
        let p = palette(t);
        let by_name = |n: &str| {
            contrast_table(t)
                .into_iter()
                .find(|x| x.name == n)
                .unwrap_or_else(|| panic!("no pair {n}"))
        };
        let pair = by_name("text/panel");
        assert_eq!((pair.fg, pair.bg), (p.text, p.panel));
        let pair = by_name("muted/raised");
        assert_eq!((pair.fg, pair.bg), (p.muted, p.raised));
        let pair = by_name("warn/panel");
        assert_eq!((pair.fg, pair.bg), (p.warn, p.panel));
    }
}

#[test]
fn contrast_bar_holds_on_worst_wallpaper() {
    // the bar backing is a panel with opacity BAR_PLATE_ALPHA; the wallpapers under it: black, white, grey, coloured
    for t in BOTH {
        let p = palette(t);
        for wall in [
            Rgb(0, 0, 0),
            Rgb(255, 255, 255),
            Rgb(128, 128, 128),
            Rgb(0x80, 0x00, 0x80),
            Rgb(0xff, 0xff, 0x00),
        ] {
            let under = composite(p.panel, BAR_PLATE_ALPHA, wall);
            for (what, fg) in [
                ("text", p.text),
                ("muted", p.muted),
                ("ok", p.ok),
                ("warn", p.warn),
                ("bad", p.bad),
            ] {
                let c = contrast(fg, under);
                assert!(
                    c >= 4.5,
                    "{t:?}: {what} of the bar on wallpaper {} gives {c:.2}:1",
                    wall.hex()
                );
            }
        }
    }
}

#[test]
fn contrast_composite_reference() {
    let (a, b) = (Rgb(0, 0, 0), Rgb(200, 100, 50));
    assert_eq!(composite(a, 1.0, b), a);
    assert_eq!(composite(a, 0.0, b), b);
    assert_eq!(composite(a, 0.5, b), Rgb(100, 50, 25));
}

#[test]
fn contrast_measure_catches_a_bad_pair() {
    // the check does not let everything through: grey #777 on white is 4.48:1, below the 4.5 threshold
    let c = contrast(Rgb(0x77, 0x77, 0x77), Rgb(255, 255, 255));
    assert!((4.4..4.5).contains(&c), "{c}");
    assert!((contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) - 21.0).abs() < 1e-9);
}

#[test]
fn contrast_bar_css_uses_plate_not_transparent() {
    let css = css();
    for t in BOTH {
        let rgba = palette(t).panel.rgba(BAR_PLATE_ALPHA);
        assert!(
            css.contains(&rgba),
            "{t:?}: no backing {rgba} in the bar CSS"
        );
    }
    assert!(!css.contains(".bar { background-color: transparent"));
}
