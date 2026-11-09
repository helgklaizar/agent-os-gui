//! S6-05: address parsing, schemes, long and hostile input, display without control characters.

use controllers::browser_addr::{
    Address, MAX_INPUT_CHARS, display_text, is_loadable, parse_address,
};

fn url(s: &str) -> Address {
    Address::Url(s.to_string())
}

fn is_search(a: &Address) -> bool {
    matches!(a, Address::Search { .. })
}

#[test]
fn parse_address_word_is_search() {
    let a = parse_address("météo");
    let Address::Search { query, url } = a else {
        panic!("a word is a search: {a:?}")
    };
    assert_eq!(query, "météo");
    assert!(url.starts_with("https://www.google.com/search?q="), "{url}");
    assert!(url.contains("%C3%A9"), "the query is encoded: {url}");
    assert!(is_search(&parse_address("rust")));
    assert!(is_search(&parse_address("x")));
}

#[test]
fn parse_address_domain_is_url() {
    assert_eq!(parse_address("example.com"), url("https://example.com"));
    assert_eq!(parse_address("  example.com  "), url("https://example.com"));
    assert_eq!(parse_address("https://x.y/z"), url("https://x.y/z"));
    assert_eq!(
        parse_address("http://x.y/z?a=1#f"),
        url("http://x.y/z?a=1#f")
    );
    assert_eq!(
        parse_address("news.ycombinator.com/ru/all"),
        url("https://news.ycombinator.com/ru/all")
    );
    assert_eq!(
        parse_address("a-b.example.org:8443/p"),
        url("https://a-b.example.org:8443/p")
    );
    assert_eq!(
        parse_address("HTTPS://Example.com"),
        url("HTTPS://Example.com")
    );
}

#[test]
fn parse_address_localhost_port_is_url() {
    assert_eq!(
        parse_address("localhost:3000"),
        url("http://localhost:3000")
    );
    assert_eq!(parse_address("localhost"), url("http://localhost"));
    assert_eq!(
        parse_address("127.0.0.1:8080/x"),
        url("http://127.0.0.1:8080/x")
    );
    assert!(
        is_search(&parse_address("localhost:99999")),
        "port out of range"
    );
    assert!(is_search(&parse_address("localhost:abc")));
}

#[test]
fn parse_address_domain_with_space_is_search() {
    assert!(is_search(&parse_address("foo.bar baz.com")));
    assert!(is_search(&parse_address("où est")));
    assert!(is_search(&parse_address("example.com\tfoo")));
}

#[test]
fn parse_address_empty_and_blank() {
    assert_eq!(parse_address(""), Address::Empty);
    assert_eq!(parse_address("   \t "), Address::Empty);
}

#[test]
fn parse_address_never_loads_other_schemes() {
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,<script>alert(1)</script>",
        "mailto:a@b.co",
        "ftp://x.y/z",
        "about:blank",
        "view-source:https://x.y",
        "vbscript:msgbox",
    ] {
        let a = parse_address(bad);
        assert!(
            is_search(&a),
            "{bad}: must go to search, not to loading: {a:?}"
        );
        if let Address::Search { url, .. } = &a {
            assert!(is_loadable(url));
        }
    }
}

#[test]
fn parse_address_hostile_input_is_search_text() {
    for bad in [
        "https://x<img src=x onerror=alert(1)>",
        "\"><script>alert(1)</script>",
        "<img src=x onerror=alert(1)>",
        "https://",
        "http:///x",
        "https://exa mple.com",
        ".com",
        "a..b.com",
        "-a.com",
    ] {
        let a = parse_address(bad);
        assert!(is_search(&a), "{bad}: {a:?}");
    }
}

#[test]
fn parse_address_long_input_is_cut() {
    let long = "é".repeat(100_000);
    let a = parse_address(&long);
    let Address::Search { query, url } = a else {
        panic!("a long string is a search")
    };
    assert_eq!(query.chars().count(), MAX_INPUT_CHARS);
    assert!(
        url.len() < 7 * MAX_INPUT_CHARS,
        "the results URL is bounded: {}",
        url.len()
    );
    let long_host = format!("https://{}.com/", "a".repeat(100_000));
    assert!(
        is_search(&parse_address(&long_host)),
        "label longer than 63 characters"
    );
}

#[test]
fn display_text_strips_controls_and_truncates() {
    assert_eq!(display_text("a\u{0}b\u{7}c\nd", 10), "abcd");
    assert_eq!(
        display_text("<b>x</b>", 20),
        "<b>x</b>",
        "markup stays as literal text"
    );
    let cut = display_text(&"x".repeat(500), 50);
    assert_eq!(cut.chars().count(), 51);
    assert!(cut.ends_with('…'));
    assert_eq!(display_text("abc", 3), "abc", "exactly at the limit");
    assert_eq!(display_text("abcd", 3), "abc…");
}

#[test]
fn is_loadable_allows_only_http_and_https() {
    assert!(is_loadable("https://x.y/"));
    assert!(is_loadable("HTTP://x.y/"));
    assert!(!is_loadable("javascript:1"));
    for bad in [
        "javascript://example.com/%0Aalert(1)",
        "data://x.y/z",
        "file://localhost/etc/passwd",
        "ftp://x.y/",
        "view-source://x.y/",
        "ws://x.y/",
    ] {
        assert!(!is_loadable(bad), "{bad}: scheme is neither http nor https");
    }
    assert!(!is_loadable("file:///x"));
    assert!(!is_loadable("https://"));
    assert!(!is_loadable(""));
    assert!(!is_loadable("https://x y"));
    assert!(!is_loadable("https://x.y/\u{0}"));
}

#[test]
fn parse_address_control_chars_without_space_go_to_search() {
    for bad in [
        "example\u{7}.com",
        "exam\u{0}ple.com",
        "https://x.y/\u{1b}[31m",
    ] {
        assert!(is_search(&parse_address(bad)), "{bad:?}");
    }
}

#[test]
fn parse_address_ipv4_needs_four_octets_up_to_255() {
    assert_eq!(
        parse_address("255.255.255.255"),
        url("http://255.255.255.255")
    );
    assert_eq!(parse_address("10.0.0.1:8080"), url("http://10.0.0.1:8080"));
    for bad in [
        "1.2.3.256",
        "1.2.3",
        "1.2.3.4.5",
        "1.2.3.",
        "999.1.1.1",
        "1.2.3.1000",
        "1.2.3.a",
        "1..3.4",
        "1.2.3.4444",
    ] {
        assert!(is_search(&parse_address(bad)), "{bad}: not an address");
    }
}

#[test]
fn parse_address_port_must_be_1_to_65535() {
    assert_eq!(
        parse_address("example.com:65535"),
        url("https://example.com:65535")
    );
    assert_eq!(parse_address("example.com:1"), url("https://example.com:1"));
    for bad in [
        "example.com:0",
        "example.com:65536",
        "example.com:",
        "example.com:123456",
        "example.com:8a",
    ] {
        assert!(is_search(&parse_address(bad)), "{bad}");
    }
}

#[test]
fn parse_address_host_length_is_bounded_to_253() {
    let host = |last: usize| {
        format!(
            "{}.{}.{}.{}.com",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(last)
        )
    };
    let ok = host(57);
    assert_eq!(ok.len(), 253);
    assert_eq!(parse_address(&ok), url(&format!("https://{ok}")));
    let long = host(58);
    assert_eq!(long.len(), 254);
    assert!(
        is_search(&parse_address(&long)),
        "254 characters is not an address"
    );
    assert!(is_search(&parse_address(":80")), "empty host name");
}

#[test]
fn parse_address_zone_without_scheme_is_letters_or_punycode() {
    assert_eq!(parse_address("example.co"), url("https://example.co"));
    assert_eq!(
        parse_address("example.xn--p1ai"),
        url("https://example.xn--p1ai")
    );
    for bad in ["example.c1", "a.b", "example.1a", "1.5"] {
        assert!(is_search(&parse_address(bad)), "{bad}");
    }
    assert_eq!(
        parse_address("http://a.b"),
        url("http://a.b"),
        "with a scheme any zone is allowed"
    );
    assert_eq!(parse_address("http://intranet"), url("http://intranet"));
}
