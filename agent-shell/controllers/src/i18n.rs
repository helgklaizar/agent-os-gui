//! Interface strings: Fluent, `assets/i18n/en.ftl` (the only language). Code holds only keys
//! (functions `tr` and `tr_args`).
//!
//! Test `i18n_all_keys_present` checks keys used in sources against `en.ftl` and looks for Cyrillic
//! literals in code. The protocol field `language` (`en`/`ru`) belongs to the agentd API; the GUI offers English only.

use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// Contents of `en.ftl`, embedded in the binary.
pub const EN_FTL: &str = include_str!("../../../assets/i18n/en.ftl");

/// Interface language: English is the only one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    En,
}

impl Lang {
    pub const ALL: [Lang; 1] = [Lang::En];

    /// Language code as in `settings.language` and `personas.lang`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
        }
    }

    /// Parses a code (case-sensitive: the core sends lowercase); an unsupported code (including `ru`) is `None`.
    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    fn source(self) -> &'static str {
        match self {
            Lang::En => EN_FTL,
        }
    }
}

fn load(lang: Lang) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = lang.code().parse().unwrap_or_default();
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    // no isolation marks U+2068/U+2069 around substitutions: GTK renders them as boxes
    bundle.set_use_isolating(false);
    // on a parse error Fluent keeps the parsed messages; completeness and cleanliness of .ftl are held by tests
    let resource = match FluentResource::try_new(lang.source().to_string()) {
        Ok(r) => r,
        Err((r, _errors)) => r,
    };
    let _ = bundle.add_resource(resource);
    bundle
}

fn bundle(lang: Lang) -> &'static FluentBundle<FluentResource> {
    static EN: OnceLock<FluentBundle<FluentResource>> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| load(Lang::En)),
    }
}

/// Current interface language (always English).
pub fn language() -> Lang {
    Lang::En
}

/// Stub: English is the only language, so there is nothing to switch.
pub fn set_language(lang: Lang) {
    match lang {
        Lang::En => {}
    }
}

/// Stub for a language code from settings; `false` means the code is unsupported (`ru` included), language untouched.
pub fn set_language_code(code: &str) -> bool {
    match Lang::from_code(code) {
        Some(lang) => {
            set_language(lang);
            true
        }
        None => false,
    }
}

/// UI string by key. An unknown key shows in the UI as `⟦key⟧`, not as an empty string.
pub fn tr(key: &str) -> String {
    tr_lang(language(), key)
}

/// String with variables substituted (`{ $name }`). A missing variable stays in the text as `{$name}`.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> String {
    tr_args_lang(language(), key, args)
}

/// String in the given language.
pub fn tr_lang(lang: Lang, key: &str) -> String {
    tr_args_lang(lang, key, &[])
}

/// String in the given language with variables substituted.
pub fn tr_args_lang(lang: Lang, key: &str, args: &[(&str, &str)]) -> String {
    let bundle = bundle(lang);
    let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) else {
        return format!("⟦{key}⟧");
    };
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, *value);
    }
    let mut errors = Vec::new();
    bundle
        .format_pattern(pattern, Some(&fluent_args), &mut errors)
        .into_owned()
}
