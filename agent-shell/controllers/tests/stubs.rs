//! Rule: "controllers do not know each other" (S1-04, S1-07).

use std::fs;
use std::path::PathBuf;

const ALL: &[(&str, &str)] = &[
    ("shell", "ShellController"),
    ("theme", "ThemeController"),
    ("connection", "ConnectionController"),
    ("bar", "BarController"),
    ("quota", "QuotaController"),
    ("metrics", "MetricsController"),
    ("indicator", "IndicatorController"),
    ("notifications", "NotificationsController"),
    ("session_tree", "SessionTreeController"),
    ("chat", "ChatController"),
    ("tools", "ToolsController"),
    ("browser", "BrowserController"),
    ("terminal", "TerminalController"),
    ("session_settings", "SessionSettingsController"),
    ("modules", "ModulesController"),
    ("settings", "SettingsController"),
    ("dialog", "DialogController"),
    ("search", "SearchController"),
    ("shortcut", "ShortcutController"),
];

fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn controllers_do_not_import_each_other() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/controllers");
    for (module, own) in ALL {
        let path = dir.join(format!("{module}.rs"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (other_mod, other) in ALL {
            if other == own {
                continue;
            }
            assert!(!has_word(&text, other), "{module}.rs references {other}");
            assert!(
                !text.contains(&format!("controllers::{other_mod}")),
                "{module}.rs imports module {other_mod}"
            );
        }
        assert!(
            !text.contains("super::"),
            "{module}.rs: `super::` is forbidden"
        );
    }
}
