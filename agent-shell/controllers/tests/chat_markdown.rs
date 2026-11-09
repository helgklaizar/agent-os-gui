//! S4-04: Markdown to a safe model, HTML escaping, link schemes.

use std::time::{Duration, Instant};

use controllers::markdown::{Block, Inline, render};
use controllers::testkit::evil_strings;

fn t(s: &str) -> Inline {
    Inline::Text(s.to_string())
}

fn link(text: &str, url: Option<&str>) -> Inline {
    Inline::Link {
        text: text.to_string(),
        url: url.map(str::to_string),
    }
}

fn inlines(blocks: &[Block]) -> Vec<&Inline> {
    blocks
        .iter()
        .flat_map(|b| match b {
            Block::Paragraph(i) | Block::Heading { inlines: i, .. } => i.iter().collect::<Vec<_>>(),
            Block::List { items, .. } => items.iter().flatten().collect(),
            Block::Code { .. } => Vec::new(),
        })
        .collect()
}

fn first_link(src: &str) -> Option<Option<String>> {
    inlines(&render(src)).into_iter().find_map(|i| match i {
        Inline::Link { url, .. } => Some(url.clone()),
        _ => None,
    })
}

#[test]
fn markdown_renders_code_lists_links() {
    let src = "# Heading\n\ntext **bold** and *italic* and `code` and [site](https://example.com)\n\n- one\n- two\n\n1. a\n2. b\n\n```rust\nlet x = 1;\nlet y = 2;\n```";
    let want = vec![
        Block::Heading {
            level: 1,
            inlines: vec![t("Heading")],
        },
        Block::Paragraph(vec![
            t("text "),
            Inline::Strong("bold".into()),
            t(" and "),
            Inline::Emph("italic".into()),
            t(" and "),
            Inline::Code("code".into()),
            t(" and "),
            link("site", Some("https://example.com")),
        ]),
        Block::List {
            ordered: false,
            items: vec![vec![t("one")], vec![t("two")]],
        },
        Block::List {
            ordered: true,
            items: vec![vec![t("a")], vec![t("b")]],
        },
        Block::Code {
            lang: Some("rust".into()),
            text: "let x = 1;\nlet y = 2;".into(),
        },
    ];
    assert_eq!(render(src), want);
}

#[test]
fn markdown_unterminated_fence_keeps_rest_as_code() {
    assert_eq!(
        render("```\none\ntwo"),
        vec![Block::Code {
            lang: None,
            text: "one\ntwo".into()
        }]
    );
}

#[test]
fn markdown_strips_html() {
    for src in [
        "<script>alert(1)</script>",
        "<b>x</b>",
        "<img src=x onerror=alert(1)>",
    ] {
        assert_eq!(
            render(src),
            vec![Block::Paragraph(vec![t(src)])],
            "HTML is shown literally: {src}"
        );
    }
    assert_eq!(
        render("```\n<script>x</script>\n```"),
        vec![Block::Code {
            lang: None,
            text: "<script>x</script>".into()
        }]
    );
    assert_eq!(
        first_link("[<b>x</b>](https://a.b)"),
        Some(Some("https://a.b".into()))
    );
}

#[test]
fn markdown_link_schemes() {
    for ok in [
        "https://a.b/c?d=1",
        "http://a.b",
        "HTTPS://A.B",
        "mailto:me@a.b",
    ] {
        assert_eq!(
            first_link(&format!("[x]({ok})")),
            Some(Some(ok.to_string())),
            "{ok}"
        );
    }
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "java\tscript:alert(1)",
        " javascript:alert(1)",
        "data:text/html,<b>",
        "vbscript:x",
        "file:///etc/passwd",
        "//evil.example",
        "/relative",
        "ftp://a.b",
        "https://a.b/ x",
        "https://a.b/\tonclick",
    ] {
        assert_eq!(
            first_link(&format!("[x]({bad})")),
            Some(None),
            "{bad:?} is not clickable"
        );
    }
}

#[test]
fn markdown_bare_dangerous_url_is_plain_text() {
    assert_eq!(first_link("javascript:alert(1)"), None);
    assert_eq!(
        render("javascript:alert(1)"),
        vec![Block::Paragraph(vec![t("javascript:alert(1)")])]
    );
}

#[test]
fn markdown_evil_strings_never_produce_html_or_unsafe_links() {
    for s in evil_strings() {
        let blocks = render(&s);
        for i in inlines(&blocks) {
            if let Inline::Link { url: Some(u), .. } = i {
                let l = u.to_lowercase();
                assert!(
                    l.starts_with("http://")
                        || l.starts_with("https://")
                        || l.starts_with("mailto:"),
                    "unsafe link {u:?}"
                );
            }
        }
        let flat: String = blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) | Block::Heading { inlines: i, .. } => i
                    .iter()
                    .map(|x| match x {
                        Inline::Text(s) | Inline::Code(s) | Inline::Strong(s) | Inline::Emph(s) => {
                            s.clone()
                        }
                        Inline::Link { text, .. } => text.clone(),
                    })
                    .collect::<String>(),
                Block::Code { text, .. } => text.clone(),
                Block::List { .. } => String::new(),
            })
            .collect();
        assert_eq!(
            flat.matches('<').count(),
            s.matches('<').count(),
            "no markup character is lost or transformed"
        );
    }
}

#[test]
fn markdown_pathological_input_is_linear() {
    let cases = [
        "[".repeat(100_000),
        "[a](".repeat(25_000),
        "[a]".repeat(30_000),
        "*".repeat(100_000),
        "`".repeat(100_000),
        "**a ".repeat(25_000),
        "# ".repeat(50_000),
        "- ".repeat(50_000),
    ];
    for src in cases {
        let start = Instant::now();
        let _ = render(&src);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "parsing {:?}… takes longer than 2 s",
            &src[..4]
        );
    }
}
