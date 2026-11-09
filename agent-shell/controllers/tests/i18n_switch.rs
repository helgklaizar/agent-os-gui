//! English is the only interface language: the language API is a stub, `ru` is not accepted,
//! strings and date format stay English. Separate test process: the file keeps the checks in one test.

use controllers::clock_fmt::format_date;
use controllers::i18n::{Lang, language, set_language, set_language_code, tr, tr_args};

const NOW: i64 = 1_793_628_309; // 2026-11-02 14:05:09 UTC, Monday

#[test]
fn i18n_english_is_the_only_language() {
    assert_eq!(language(), Lang::En, "English by default");
    assert_eq!(tr("column-chat"), "Chat");
    assert_eq!(format_date(NOW, 7200), "Mon, Nov 2");

    set_language(Lang::En);
    assert_eq!(language(), Lang::En);
    assert_eq!(
        (
            tr("column-chat"),
            tr_args("bar-theme-button", &[("theme", "Dark")]),
            format_date(NOW, 7200),
        ),
        (
            "Chat".to_string(),
            "Theme: Dark".to_string(),
            "Mon, Nov 2".to_string()
        ),
        "strings and date format by locale"
    );

    // language code from settings: only `en` is accepted, others do not change the language
    assert!(set_language_code("en"));
    for bad in ["ru", "de", "EN", ""] {
        assert!(!set_language_code(bad), "{bad:?}");
        assert_eq!(language(), Lang::En, "{bad:?} must not change the language");
    }
    assert_eq!(tr("column-chat"), "Chat");
    assert_eq!(format_date(NOW, 7200), "Mon, Nov 2");
}
