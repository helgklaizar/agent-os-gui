//! Localization: `en.ftl` well-formedness, no Cyrillic, language codes.

use std::collections::BTreeMap;

use controllers::i18n::{EN_FTL, Lang, tr_args_lang, tr_lang};
use fluent_bundle::FluentResource;
use fluent_syntax::ast::{Entry, Expression, InlineExpression, Pattern, PatternElement};

fn messages(src: &str) -> BTreeMap<String, Vec<String>> {
    let res = FluentResource::try_new(src.to_string())
        .unwrap_or_else(|(_, e)| panic!("parse errors: {e:?}"));
    let mut out = BTreeMap::new();
    for entry in res.entries() {
        if let Entry::Message(m) = entry {
            let mut vars = Vec::new();
            if let Some(p) = &m.value {
                collect(p, &mut vars);
            }
            vars.sort();
            vars.dedup();
            out.insert(m.id.name.to_string(), vars);
        }
    }
    out
}

fn collect(p: &Pattern<&str>, out: &mut Vec<String>) {
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            inline_vars(expression, out);
        }
    }
}

fn inline_vars(e: &Expression<&str>, out: &mut Vec<String>) {
    match e {
        Expression::Inline(i) => inline(i, out),
        Expression::Select { selector, variants } => {
            inline(selector, out);
            for v in variants {
                collect(&v.value, out);
            }
        }
    }
}

fn inline(i: &InlineExpression<&str>, out: &mut Vec<String>) {
    match i {
        InlineExpression::VariableReference { id } => out.push(id.name.to_string()),
        InlineExpression::Placeable { expression } => inline_vars(expression, out),
        _ => {}
    }
}

#[test]
fn i18n_en_messages_are_well_formed() {
    let en = messages(EN_FTL);
    assert!(!en.is_empty(), "en.ftl is empty");
    // every message is reachable through `tr_lang` and has no unresolved key marker
    for k in en.keys() {
        let e = tr_lang(Lang::En, k);
        assert!(!e.contains('\u{27e6}'), "{k}: missing in en");
    }
}

#[test]
fn i18n_en_has_no_cyrillic() {
    let cyr = |t: &str| t.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let en = messages(EN_FTL);
    let mut checked = 0;
    for k in en.keys() {
        let e = tr_lang(Lang::En, k);
        assert!(!cyr(&e), "{k}: Cyrillic in en: {e}");
        checked += 1;
    }
    assert!(checked >= 400, "keys checked: {checked}");
    assert!(!cyr(EN_FTL), "Cyrillic in en.ftl");
}

#[test]
fn i18n_lang_in_en_example() {
    assert_eq!(tr_lang(Lang::En, "column-sessions"), "Sessions");
    assert_eq!(tr_lang(Lang::En, "set-lang-en"), "English");
    assert_eq!(
        tr_args_lang(Lang::En, "bar-theme-button", &[("theme", "Dark")]),
        "Theme: Dark"
    );
    assert_eq!(tr_lang(Lang::En, "no-such-key"), "⟦no-such-key⟧");
}

#[test]
fn i18n_lang_codes() {
    assert_eq!(Lang::En.code(), "en");
    assert_eq!(Lang::from_code("en"), Some(Lang::En));
    // `ru` stays a valid protocol value but the GUI has no Russian interface
    for bad in ["", "EN", "de", "en-US", "ru", "ru "] {
        assert_eq!(Lang::from_code(bad), None, "{bad:?}");
    }
}
