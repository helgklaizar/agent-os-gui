//! Accessible names, checked against the sources: GTK is absent on macOS, but the rule can be checked from the text.
//!
//! Rules: (1) an icon button is created only via `a11y::icon_button` (name and tooltip are set together there);
//! (2) every `Button`/`ToggleButton`/`MenuButton` created via `::new()` and given an icon (`set_icon_name`) gets `a11y::set_name(&it, …)` nearby;
//! (3) an empty name `""` in `icon_button` is allowed only if the file later sets the name via `a11y::set_name`.
//! Walking the real accessibility tree on live GTK is done by the `*_a11y` scenarios in `tests/scenarios` (Linux).

use std::fs;
use std::path::{Path, PathBuf};

fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")) {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            (p, text)
        })
        .collect()
}

/// Rule violations in one file; `file` is the file name for messages and the `a11y.rs` exception.
fn violations(file: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let code = |l: &str| l.split("//").next().unwrap_or("").to_string();
    for (i, raw) in lines.iter().enumerate() {
        let line = code(raw);
        if file != "a11y.rs" && line.contains("Button::from_icon_name") {
            out.push(format!(
                "{file}:{}: icon button bypasses a11y::icon_button",
                i + 1
            ));
        }
        let decl = line
            .trim()
            .strip_prefix("let ")
            .and_then(|r| r.split_once(" = "));
        if let Some((var, init)) = decl
            && ["Button::new()", "ToggleButton::new()", "MenuButton::new()"]
                .iter()
                .any(|k| init.contains(k))
        {
            let var = var.trim_start_matches("mut ").trim();
            let window = &lines[i..lines.len().min(i + 40)];
            let has_icon = window
                .iter()
                .any(|l| code(l).contains(&format!("{var}.set_icon_name(")));
            let named = window.iter().any(|l| {
                let l = code(l);
                l.contains(&format!("set_name(&{var}")) || l.contains(&format!("set_name({var}"))
            });
            if has_icon && !named {
                out.push(format!(
                    "{file}:{}: icon button `{var}` has no a11y::set_name in the next 40 lines",
                    i + 1
                ));
            }
        }
        if line.contains("icon_button(") && line.contains(", \"\")") && !text.contains("set_name(")
        {
            out.push(format!("{file}:{}: empty icon button name", i + 1));
        }
    }
    out
}

#[test]
fn every_icon_button_goes_through_a11y_helper_and_is_named() {
    let mut found = Vec::new();
    let all = sources();
    assert!(all.len() > 20, "ui sources not found: {}", all.len());
    for (path, text) in &all {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        found.extend(violations(name, text));
    }
    assert!(
        found.is_empty(),
        "buttons without a name:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_probe_reports_unnamed_and_raw_icon_buttons() {
    // the check does not let everything through: deliberately bad text must produce violations
    let bad = "fn f() {\n    let b = gtk::Button::from_icon_name(\"x\");\n    let t = gtk::ToggleButton::new();\n    t.set_icon_name(\"y\");\n}\n";
    let v = violations("bad.rs", bad);
    assert_eq!(v.len(), 2, "{v:?}");
    let empty = "let b = a11y::icon_button(\"x\", \"\");\n";
    assert_eq!(violations("empty.rs", empty).len(), 1);
    let good =
        "let t = gtk::ToggleButton::new();\nt.set_icon_name(\"y\");\na11y::set_name(&t, &name);\n";
    assert!(violations("good.rs", good).is_empty());
    // a comment does not count as code
    assert!(violations("c.rs", "// let b = Button::from_icon_name(\"x\");\n").is_empty());
    // a11y.rs is the only place where an icon is assembled directly
    assert!(violations("a11y.rs", "let b = gtk::Button::from_icon_name(icon);\n").is_empty());
}
