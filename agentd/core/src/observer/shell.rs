//! Rough command-line parsing for observer rules: parts, program, arguments, flags, redirections.
//! This is not a shell: substitutions and escaping are not expanded, parsing is only needed to match the rules.

/// One part of a command between separators.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Segment {
    pub program: String,
    /// Arguments without flags (not starting with `-`).
    pub args: Vec<String>,
    /// Letters of short flags and long flags reduced to letters (`--recursive` → `r`, `--force` → `f`).
    pub flags: Vec<char>,
    pub redirects: Vec<String>,
    /// The part passes its output to the next one via `|`.
    pub piped_to_next: bool,
}

const WRAPPERS: [&str; 6] = ["sudo", "env", "nohup", "time", "command", "exec"];

fn unquote(t: &str) -> String {
    t.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// The signs `$HOME` and `${HOME}` at the start of a word equal `~`.
fn home(t: String) -> String {
    for p in ["${HOME}", "$HOME"] {
        if let Some(rest) = t.strip_prefix(p) {
            return format!("~{rest}");
        }
    }
    t
}

/// Words: the separator is a space outside quotes; `;`, `&`, `|`, newline and `>` outside quotes are separate tokens.
fn words(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => quote = Some(c),
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (None, c) if c.is_whitespace() && c != '\n' => flush(&mut cur, &mut out),
            (None, '\n' | ';') => {
                flush(&mut cur, &mut out);
                out.push(";".into());
            }
            (None, '|' | '&') => {
                flush(&mut cur, &mut out);
                let mut op = c.to_string();
                if chars.peek() == Some(&c) {
                    chars.next();
                    op.push(c);
                }
                out.push(op);
            }
            (None, '>') => {
                flush(&mut cur, &mut out);
                let mut op = ">".to_string();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    op.push('>');
                }
                out.push(op);
            }
            (None, c) => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn long_flag(f: &str) -> Option<char> {
    match f {
        "--recursive" => Some('r'),
        "--force" => Some('f'),
        _ => None,
    }
}

fn segment(ws: &[String], piped: bool) -> Segment {
    let mut seg = Segment {
        piped_to_next: piped,
        ..Segment::default()
    };
    let mut i = 0;
    let mut want_redirect = false;
    // Wrappers (`sudo`, `env`…), their flags and `X=y` assignments come before the program.
    let mut wrapped = false;
    while i < ws.len() {
        let w = &ws[i];
        let is_assign = w.contains('=') && !w.starts_with('-') && !w.starts_with('/');
        if is_assign || WRAPPERS.contains(&w.as_str()) {
            wrapped = true;
        } else if !(wrapped && w.starts_with('-')) {
            break;
        }
        i += 1;
    }
    for w in &ws[i..] {
        if w == ">" || w == ">>" {
            want_redirect = true;
            continue;
        }
        let w = home(unquote(w));
        if want_redirect {
            seg.redirects.push(w);
            want_redirect = false;
        } else if seg.program.is_empty() {
            seg.program = w.rsplit('/').next().unwrap_or_default().to_string();
        } else if let Some(long) = w.strip_prefix("--") {
            if let Some(c) = long_flag(&format!("--{long}")) {
                seg.flags.push(c);
            }
        } else if w.starts_with('-') && w.len() > 1 {
            seg.flags.extend(w[1..].chars());
        } else {
            seg.args.push(w);
        }
    }
    seg
}

/// Command parts in written order.
pub(super) fn parse(cmd: &str) -> Vec<Segment> {
    let ws = words(cmd);
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for w in ws {
        match w.as_str() {
            ";" | "&" | "&&" | "||" => {
                out.push(segment(&cur, false));
                cur.clear();
            }
            "|" => {
                out.push(segment(&cur, true));
                cur.clear();
            }
            _ => cur.push(w),
        }
    }
    out.push(segment(&cur, false));
    out.retain(|s| !s.program.is_empty());
    out
}
