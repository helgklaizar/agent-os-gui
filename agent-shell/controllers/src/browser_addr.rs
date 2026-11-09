//! Browser address-bar parsing: a word is a search, a domain or `localhost:port` is an address; schemes other than `http` and `https`
//! are never loaded. The functions are pure: no network and no GTK.

/// What to do with the entered string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// Empty: do nothing.
    Empty,
    /// Load the address (`http` or `https`).
    Url(String),
    /// Search: the query string and the ready-made search results URL.
    Search { query: String, url: String },
}

/// At most this many characters are accepted in the address bar (the rest is discarded).
pub const MAX_INPUT_CHARS: usize = 2048;

const SEARCH_BASE: &str = "https://www.google.com/search?q=";

/// Parses the entered string.
pub fn parse_address(input: &str) -> Address {
    let t = input.trim();
    if t.is_empty() {
        return Address::Empty;
    }
    if t.chars().count() <= MAX_INPUT_CHARS
        && !t.chars().any(|c| c.is_whitespace() || c.is_control())
        && let Some(url) = as_url(t)
    {
        return Address::Url(url);
    }
    let query: String = t.chars().take(MAX_INPUT_CHARS).collect();
    let url = format!("{SEARCH_BASE}{}", encode(&query));
    Address::Search { query, url }
}

/// The address from a string without spaces, or `None` if it is not an address.
fn as_url(t: &str) -> Option<String> {
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let rest = &t[lower.find("://")? + 3..];
        return host_port_ok(authority(rest), true).then(|| t.to_string());
    }
    let auth = authority(t);
    if !host_port_ok(auth, false) {
        return None;
    }
    let host = auth.rsplit_once(':').map_or(auth, |(h, _)| h);
    let scheme = if host == "localhost" || is_ipv4(host) {
        "http"
    } else {
        "https"
    };
    Some(format!("{scheme}://{t}"))
}

/// The part before the first `/`, `?` or `#`.
fn authority(s: &str) -> &str {
    s.find(['/', '?', '#']).map_or(s, |i| &s[..i])
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn label_ok(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= 63
        && !l.starts_with('-')
        && !l.ends_with('-')
        && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `host[:port]`; `explicit` means the scheme was already entered, so a single-label name is allowed (`http://intranet`).
fn host_port_ok(auth: &str, explicit: bool) -> bool {
    let (host, port) = match auth.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (auth, None),
    };
    if let Some(p) = port
        && !(!p.is_empty()
            && p.len() <= 5
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n)))
    {
        return false;
    }
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") || is_ipv4(host) {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if !labels.iter().all(|l| label_ok(l)) {
        return false;
    }
    if labels.len() == 1 {
        return explicit;
    }
    // without a scheme the zone is letters (at least two) or punycode: this tells `example.com` apart from `1.5` and `a.b`
    let tld = labels[labels.len() - 1];
    explicit
        || tld.starts_with("xn--")
        || (tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// UTF-8 percent-encoding: only `A-Z a-z 0-9 - . _ ~` are kept as is.
fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 3);
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Text for display: control characters removed, length at most `max` characters (the rest is replaced by "…"). No markup: display only
/// via `set_label` with `set_use_markup(false)` or an input field.
pub fn display_text(raw: &str, max: usize) -> String {
    let mut out = String::new();
    for (n, c) in raw.chars().filter(|c| !c.is_control()).enumerate() {
        if n == max {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// The address may be loaded in the browser: only `http` and `https`.
pub fn is_loadable(url: &str) -> bool {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let Some(i) = lower.find("://") else {
        return false;
    };
    matches!(&lower[..i], "http" | "https") && host_port_ok(authority(&url[i + 3..]), true)
}
