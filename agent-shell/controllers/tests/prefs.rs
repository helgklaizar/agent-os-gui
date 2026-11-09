//! Tests of the `ui.json` format (`ui_prefs_roundtrip`, `ui_prefs_corrupt_falls_back`).

use controllers::prefs::{ColumnsPrefs, UiFile, UiPrefs, WindowPrefs};
use controllers::{Manual, Theme};

#[test]
fn ui_prefs_roundtrip() {
    let mut f = UiFile {
        theme: Theme::Dark,
        ..UiFile::default()
    };
    f.columns.tools.manual = Manual::Expanded;
    f.columns.notifications.manual = Manual::Collapsed;
    f.window = WindowPrefs { w: 1500, h: 900 };
    let (back, warn) = UiFile::parse_or_default(&f.to_json());
    assert_eq!(back, f);
    assert_eq!(warn, None);
}

#[test]
fn ui_prefs_matches_documented_format() {
    let text = r#"{ "version": 1, "theme": "auto",
      "columns": { "notifications": {"manual": null}, "sessions": {"manual": null}, "tools": {"manual": "collapsed"} },
      "window": { "w": 1280, "h": 800 } }"#;
    let (f, warn) = UiFile::parse_or_default(text);
    assert_eq!(warn, None);
    assert_eq!(f.theme, Theme::Auto);
    assert_eq!(f.columns.tools.manual, Manual::Collapsed);
    assert_eq!(f.columns.sessions.manual, Manual::Auto);

    let json: serde_json::Value = serde_json::from_str(&UiFile::default().to_json()).expect("json");
    assert_eq!(json["version"], 1);
    assert_eq!(json["theme"], "auto");
    assert!(json["columns"]["tools"]["manual"].is_null());
}

#[test]
fn ui_prefs_corrupt_falls_back() {
    for text in ["", "{", "not json", "[]", r#"{"version":1,"theme":"neon"}"#] {
        let (f, warn) = UiFile::parse_or_default(text);
        assert_eq!(f, UiFile::default(), "text={text:?}");
        assert!(warn.is_some(), "a log reason is required: {text:?}");
    }
}

#[test]
fn ui_prefs_unknown_version_falls_back() {
    let (f, warn) = UiFile::parse_or_default(r#"{"version":2,"theme":"dark"}"#);
    assert_eq!(f, UiFile::default());
    assert!(warn.is_some());
}

#[test]
fn ui_prefs_partial_update_keeps_other_fields() {
    let mut f = UiFile::default();
    f.columns.tools.manual = Manual::Expanded;
    f.apply(&UiPrefs {
        theme: Some(Theme::Light),
        ..UiPrefs::default()
    });
    assert_eq!(f.theme, Theme::Light);
    assert_eq!(f.columns.tools.manual, Manual::Expanded);

    let cols = ColumnsPrefs::default();
    f.apply(&UiPrefs {
        columns: Some(cols),
        ..UiPrefs::default()
    });
    assert_eq!(f.columns, cols);
    assert_eq!(f.theme, Theme::Light);
}
