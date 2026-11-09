//! `keys.env` format: managed entries (`# ref=… updated_at=…` and a `NAME="value"` line) and foreign lines.

use agentd_core::valid_secret_ref;

/// File line: a managed entry or foreign text.
pub(crate) enum Line {
    Entry {
        secret_ref: String,
        updated_at: i64,
        value: String,
    },
    Foreign(String),
}

pub(crate) fn var_name(secret_ref: &str) -> String {
    secret_ref
        .bytes()
        .map(|b| match b {
            b'.' | b'-' => '_',
            b => char::from(b.to_ascii_uppercase()),
        })
        .collect()
}

fn escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unescape(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match it.next()? {
            'n' => '\n',
            'r' => '\r',
            e @ ('\\' | '"' | '$' | '`') => e,
            _ => return None,
        });
    }
    Some(out)
}

pub(crate) fn parse(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let managed = line.strip_prefix("# ref=").and_then(|rest| {
            let (r, ts) = rest.split_once(" updated_at=")?;
            let value = lines.peek()?.split_once('=')?;
            (valid_secret_ref(r) && value.0 == var_name(r))
                .then(|| Some((r.to_string(), ts.parse::<i64>().ok()?, unescape(value.1)?)))
                .flatten()
        });
        if let Some((secret_ref, updated_at, value)) = managed {
            lines.next();
            out.push(Line::Entry {
                secret_ref,
                updated_at,
                value,
            });
        } else {
            out.push(Line::Foreign(line.to_string()));
        }
    }
    out
}

pub(crate) fn render(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        match l {
            Line::Foreign(t) => out.push_str(t),
            Line::Entry {
                secret_ref,
                updated_at,
                value,
            } => {
                out.push_str(&format!(
                    "# ref={secret_ref} updated_at={updated_at}\n{}={}",
                    var_name(secret_ref),
                    escape(value)
                ));
            }
        }
        out.push('\n');
    }
    out
}
