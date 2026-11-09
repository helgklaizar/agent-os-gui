//! Phishing marker in incoming text: the link label looks like an address but leads to a different host.
//! Heuristic: missed text stays `untrusted`.

/// Host from an address or label: without scheme, `www.`, path, port and case; `None` if it does not look like an address.
fn host_of(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let host = rest
        .split(['/', '?', '#', ':', ' '])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let looks_like_host = host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    looks_like_host.then_some(host)
}

/// The label is an address (`https://...` or `domain.zone`) and the target is a different host.
fn mismatch(label: &str, target: &str) -> bool {
    match (host_of(label), host_of(target)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

/// `[label](address)` in Markdown.
fn markdown_links(body: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else { break };
        let (label, tail) = (&after[..close], &after[close + 2..]);
        let Some(end) = tail.find(')') else { break };
        if !label.contains('[') {
            out.push((label, &tail[..end]));
        }
        rest = &tail[end + 1..];
    }
    out
}

/// `<a href="address">label</a>` in HTML (double or single quotes).
fn html_links(body: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("href=") {
        let tail = &rest[at + 5..];
        let Some(q) = tail.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            rest = tail;
            continue;
        };
        let inner = &tail[1..];
        let Some(end) = inner.find(q) else { break };
        let target = &inner[..end];
        let after = &inner[end + 1..];
        let Some(gt) = after.find('>') else { break };
        let text = &after[gt + 1..];
        let label = text.split("</a>").next().unwrap_or("");
        out.push((label, target));
        rest = &after[gt + 1..];
    }
    out
}

/// First "label vs different host" pair; the finding text holds only hosts, no message body.
#[must_use]
pub fn find_mismatch(body: &str) -> Option<String> {
    markdown_links(body)
        .into_iter()
        .chain(html_links(body))
        .find(|(label, target)| mismatch(label, target))
        .map(|(label, target)| {
            format!(
                "label {} leads to {}",
                host_of(label).unwrap_or_default(),
                host_of(target).unwrap_or_default()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::find_mismatch;

    #[test]
    fn flags_label_host_differs_from_target() {
        assert!(find_mismatch("[https://bank.example.com](https://evil.example.net/x)").is_some());
        assert!(find_mismatch("[bank.example.com](http://evil.example.net)").is_some());
        assert!(
            find_mismatch(r#"<a href="https://evil.example.net/">https://bank.example.com</a>"#)
                .is_some()
        );
    }

    #[test]
    fn allows_matching_and_plain_labels() {
        assert!(find_mismatch("[https://example.com/a](https://www.example.com/b)").is_none());
        assert!(find_mismatch("[our site](https://example.com/)").is_none());
        assert!(find_mismatch("just https://example.org/ and [x](y").is_none());
        assert!(find_mismatch("").is_none());
        assert!(find_mismatch("[](").is_none());
    }
}
