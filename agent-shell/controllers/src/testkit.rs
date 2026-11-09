//! Test utilities for `controllers` and `ui` (the `testkit` feature, S1-07).

#[cfg(feature = "testkit")]
/// Hostile strings for escaping checks: markup, attributes, entities, URL schemes, length, newlines, RTL.
pub fn evil_strings() -> Vec<String> {
    vec![
        "<img src=x onerror=alert(1)>".to_string(),
        "\" onmouseover=\"x".to_string(),
        "<b>x</b>".to_string(),
        "&amp;".to_string(),
        "javascript:alert(1)".to_string(),
        "<".repeat(100_000),
        "line1\nline2\r\nline3\n<b>\n".to_string(),
        "\u{202E}<script>\u{202C} שלום עולם & مرحبا".to_string(),
    ]
}
