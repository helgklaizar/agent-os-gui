//! i18n tests: keys used in code exist in `en.ftl`, no Cyrillic literals in code, `tr` returns text.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use controllers::i18n::{EN_FTL, tr, tr_args};
use fluent_bundle::FluentResource;

fn crate_src(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("directory {dir:?} unreadable: {e}"));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn product_sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for krate in ["controllers", "ui"] {
        rust_files(&crate_src(krate), &mut files);
    }
    assert!(!files.is_empty(), "controllers and ui sources not found");
    files
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            (p, text)
        })
        .collect()
}

/// String literals of Rust code without comments (plain and raw `r#"…"#`; char literals are skipped).
fn string_literals(src: &str) -> Vec<String> {
    let c: Vec<char> = src.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < c.len() {
        match c[i] {
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            'r' if matches!(c.get(i + 1), Some('"' | '#'))
                && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while c.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if c.get(j) != Some(&'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                let start = j;
                let closing: String = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                while j < c.len() && !c[j..].starts_with(&closing.chars().collect::<Vec<_>>()) {
                    j += 1;
                }
                out.push(c[start..j.min(c.len())].iter().collect());
                i = j + closing.len();
            }
            '"' => {
                let mut j = i + 1;
                let mut lit = String::new();
                while j < c.len() && c[j] != '"' {
                    if c[j] == '\\' && j + 1 < c.len() {
                        lit.push(c[j]);
                        j += 1;
                    }
                    lit.push(c[j]);
                    j += 1;
                }
                out.push(lit);
                i = j + 1;
            }
            '\'' => {
                // 'x' or '\n' is a char literal; otherwise it is a lifetime
                if c.get(i + 1) == Some(&'\\') {
                    i += 2;
                    while i < c.len() && c[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if c.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Keys from `tr("key")` and `tr_args("key", …)` calls.
fn keys_in(src: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for name in ["tr", "tr_args"] {
        let pat = format!("{name}(");
        let mut from = 0;
        while let Some(pos) = src[from..].find(&pat) {
            let at = from + pos;
            from = at + pat.len();
            let prev = src[..at].chars().next_back();
            if prev.is_some_and(|p| p.is_alphanumeric() || p == '_') {
                continue;
            }
            let rest = src[from..].trim_start();
            if let Some(body) = rest.strip_prefix('"')
                && let Some(end) = body.find('"')
            {
                keys.insert(body[..end].to_string());
            }
        }
    }
    // language variants: `tr_lang(lang, "key")` and `tr_args_lang(lang, "key", …)`, the key is the second argument
    for name in ["tr_lang", "tr_args_lang"] {
        let pat = format!("{name}(");
        let mut from = 0;
        while let Some(pos) = src[from..].find(&pat) {
            let at = from + pos;
            from = at + pat.len();
            let prev = src[..at].chars().next_back();
            if prev.is_some_and(|p| p.is_alphanumeric() || p == '_') {
                continue;
            }
            let Some((_, after_comma)) = src[from..].split_once(',') else {
                continue;
            };
            if let Some(body) = after_comma.trim_start().strip_prefix('"')
                && let Some(end) = body.find('"')
            {
                keys.insert(body[..end].to_string());
            }
        }
    }
    keys
}

/// Message keys of `en.ftl` (lines `key = …` at line start).
fn ftl_keys() -> BTreeSet<String> {
    EN_FTL
        .lines()
        .filter(|l| !l.starts_with(['#', ' ', '\t', '.', '[', '*']))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
        .filter(|k| !k.is_empty() && !k.starts_with('-'))
        .collect()
}

fn code_keys() -> BTreeSet<String> {
    product_sources()
        .iter()
        .flat_map(|(_, text)| keys_in(text))
        .collect()
}

#[test]
fn i18n_all_keys_present() {
    let used = code_keys();
    assert!(
        used.len() >= 10,
        "too few tr(\"…\") keys found in controllers and ui code: {used:?}"
    );
    let known = ftl_keys();
    let missing: Vec<_> = used.difference(&known).collect();
    assert!(
        missing.is_empty(),
        "keys used in code but missing from en.ftl: {missing:?}"
    );
}

#[test]
fn i18n_ftl_has_no_unused_keys() {
    let used = code_keys();
    let unused: Vec<_> = ftl_keys().difference(&used).cloned().collect();
    assert!(
        unused.is_empty(),
        "en.ftl keys not used in code: {unused:?}"
    );
}

#[test]
fn i18n_en_ftl_parses_without_errors() {
    let parsed = FluentResource::try_new(EN_FTL.to_string());
    assert!(parsed.is_ok(), "en.ftl failed to parse: {:?}", parsed.err());
    assert!(!ftl_keys().is_empty(), "en.ftl has no messages");
}

#[test]
fn i18n_tr_known_key() {
    assert_eq!(tr("app-name"), "Agent OS");
    assert_eq!(tr("column-notifications"), "Notifications");
}

#[test]
fn i18n_tr_args_substituted_without_isolation_marks() {
    let text = tr_args("bar-theme-button", &[("theme", "Auto")]);
    assert_eq!(text, "Theme: Auto");
    assert!(
        !text.contains(['\u{2068}', '\u{2069}']),
        "Fluent isolation marks left in the string: {text:?}"
    );
}

#[test]
fn i18n_tr_unknown_key_is_visible() {
    let text = tr("no-such-key-for-test");
    assert!(
        text.contains("no-such-key-for-test"),
        "an unknown key must not yield an empty string: {text:?}"
    );
}

#[test]
fn i18n_tr_args_missing_variable_is_visible() {
    let text = tr_args("bar-theme-button", &[]);
    assert!(
        text.starts_with("Theme:") && !text.trim_end().ends_with(':'),
        "a missing variable must not yield a truncated string: {text:?}"
    );
}

fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

#[test]
fn no_cyrillic_literals_in_controllers_and_ui() {
    let mut offenders = Vec::new();
    for (path, text) in product_sources() {
        for lit in string_literals(&text) {
            if has_cyrillic(&lit) {
                offenders.push(format!("{}: {lit}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Cyrillic literals in code (UI strings only via en.ftl):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn scanner_probe_literals_and_keys() {
    // Cyrillic is written as escapes so that this file itself stays Cyrillic-free
    let word = "\u{43b}\u{438}\u{442}"; // three letters
    let raw = "\u{441}\u{44b}\u{440} \u{ab}x\u{bb}"; // raw literal text
    let sample =
        "// COMMENT\nlet a = tr(\"k-one\"); /* BLOCK */ let b = tr_args( \"k-two\", &[]);\n\
                  let c = \"WORD\"; let d = r#\"RAW\"#; let e = 'x'; let f = mytr(\"not-key\");\n\
                  fn g<'a>(x: &'a str) {} let h = \"a\\\"b\";"
            .replace(
                "COMMENT",
                "\u{43a}\u{43e}\u{43c}\u{43c}\u{435}\u{43d}\u{442}",
            )
            .replace("BLOCK", "\u{431}\u{43b}\u{43e}\u{43a}")
            .replace("WORD", word)
            .replace("RAW", raw);
    let lits = string_literals(&sample);
    assert!(lits.contains(&word.to_string()), "{lits:?}");
    assert!(lits.contains(&raw.to_string()), "{lits:?}");
    assert!(lits.iter().any(|l| l.contains('b')), "{lits:?}");
    assert!(
        !lits
            .iter()
            .any(|l| l.contains('\u{43a}') || l.contains('\u{431}')),
        "comments are not literals: {lits:?}"
    );
    let keys = keys_in(&sample);
    assert_eq!(
        keys,
        BTreeSet::from(["k-one".to_string(), "k-two".to_string()]),
        "mytr(…) is not a key"
    );
    assert!(has_cyrillic("a \u{44f}") && !has_cyrillic("abc"));
}
