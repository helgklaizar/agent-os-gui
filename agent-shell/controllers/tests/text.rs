//! `controllers::text::escape_markup` on `testkit::evil_strings()` (S1-07).

use controllers::testkit::evil_strings;
use controllers::text::escape_markup;

/// Reverse replacement; an entity not in the list counts as an escaping error.
fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let (entity, ch) = [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&#39;", '\''),
        ]
        .into_iter()
        .find(|(e, _)| rest.starts_with(e))?;
        out.push(ch);
        rest = &rest[entity.len()..];
    }
    out.push_str(rest);
    Some(out)
}

#[test]
fn escape_markup_evil_strings() {
    let evil = evil_strings();
    assert_eq!(evil.len(), 8, "the spec set: eight strings");
    for input in &evil {
        let shown: String = input.chars().take(40).collect();
        let out = escape_markup(input);
        assert!(!out.contains('<'), "`<` left over: {shown:?}");
        assert!(!out.contains('>'), "`>` left over: {shown:?}");
        assert!(!out.contains('"'), "double quote left over: {shown:?}");
        assert!(!out.contains('\''), "apostrophe left over: {shown:?}");
        assert_eq!(
            unescape(&out).as_deref(),
            Some(input.as_str()),
            "`&` only as part of known entities, reverse replacement yields the input: {shown:?}"
        );
    }
}

#[test]
fn escape_markup_each_special_char() {
    for (input, want) in [
        ("&", "&amp;"),
        ("<", "&lt;"),
        (">", "&gt;"),
        ("\"", "&quot;"),
        ("'", "&#39;"),
        ("&amp;", "&amp;amp;"),
        ("a<b>&\"c'", "a&lt;b&gt;&amp;&quot;c&#39;"),
    ] {
        assert_eq!(escape_markup(input), want, "input {input:?}");
    }
}

#[test]
fn escape_markup_keeps_safe_text_byte_for_byte() {
    for input in [
        "",
        "plain text 123",
        "line\nwith a break\r\nof lines",
        "שלום עולם",
        "javascript:alert(1)",
        "🙂 emoji",
    ] {
        assert_eq!(escape_markup(input), input, "safe text is unchanged");
    }
}

#[test]
fn escape_markup_long_string_is_linear_and_complete() {
    let evil = evil_strings();
    let long = evil
        .iter()
        .find(|s| s.chars().count() >= 100_000)
        .expect("the set contains a 100k-character string");
    let out = escape_markup(long);
    assert_eq!(out.len(), long.len() * "&lt;".len() / "<".len());
}
