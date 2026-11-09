//! Markdown → safe model. Skeleton: the whole text as a single paragraph.

/// Inline fragment: only text, code and links; raw HTML cannot be expressed by the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(String),
    Emph(String),
    /// `url = None`: the scheme is forbidden, the link is not clickable, the text is shown.
    Link {
        text: String,
        url: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Inline>>,
    },
}

/// A link is clickable only with the `http`, `https` or `mailto` scheme; whitespace and control characters inside reject it entirely
/// (browsers read `java\tscript:` as `javascript:`).
pub fn safe_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = url.split_once(':')?;
    let allowed = ["http", "https", "mailto"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s));
    (allowed && !rest.is_empty()).then(|| url.to_string())
}

fn find(c: &[char], from: usize, ch: char) -> Option<usize> {
    c.get(from..)?
        .iter()
        .position(|x| *x == ch)
        .map(|p| p + from)
}

fn find_pair(c: &[char], from: usize, ch: char) -> Option<usize> {
    let tail = c.get(from..)?;
    tail.windows(2)
        .position(|w| w[0] == ch && w[1] == ch)
        .map(|p| p + from)
}

fn text_of(c: &[char]) -> String {
    c.iter().collect()
}

/// "No closing marker ahead" flags: after the first failure the search of that kind is not repeated (linear time on garbage).
#[derive(Default)]
struct Gone {
    tick: bool,
    strong: bool,
    emph: bool,
    close: bool,
    paren: bool,
}

fn flush(out: &mut Vec<Inline>, buf: &mut String) {
    if !buf.is_empty() {
        out.push(Inline::Text(std::mem::take(buf)));
    }
}

/// `[text](url)` with `[` at position `i`: (text, url, position after `)`).
fn link_at(
    c: &[char],
    i: usize,
    gone: &mut Gone,
    next_close: &mut Option<usize>,
) -> Option<(String, Option<String>, usize)> {
    let close = match *next_close {
        Some(j) if j > i => j,
        _ if gone.close => return None,
        _ => match find(c, i + 1, ']') {
            Some(j) => {
                *next_close = Some(j);
                j
            }
            None => {
                gone.close = true;
                return None;
            }
        },
    };
    if c.get(close + 1) != Some(&'(') || gone.paren {
        return None;
    }
    let Some(end) = find(c, close + 2, ')') else {
        gone.paren = true;
        return None;
    };
    Some((
        text_of(&c[i + 1..close]),
        safe_url(&text_of(&c[close + 2..end])),
        end + 1,
    ))
}

fn inline_line(line: &str) -> Vec<Inline> {
    let c: Vec<char> = line.chars().collect();
    let (mut out, mut buf, mut i) = (Vec::new(), String::new(), 0);
    let (mut gone, mut next_close) = (Gone::default(), None);
    while i < c.len() {
        let two = c.get(i + 1) == Some(&'*');
        match c[i] {
            '`' if !gone.tick => match find(&c, i + 1, '`') {
                Some(j) if j > i + 1 => {
                    flush(&mut out, &mut buf);
                    out.push(Inline::Code(text_of(&c[i + 1..j])));
                    i = j + 1;
                    continue;
                }
                Some(_) => {}
                None => gone.tick = true,
            },
            '*' if two && !gone.strong => match find_pair(&c, i + 2, '*') {
                Some(j) if j > i + 2 => {
                    flush(&mut out, &mut buf);
                    out.push(Inline::Strong(text_of(&c[i + 2..j])));
                    i = j + 2;
                    continue;
                }
                Some(_) => {}
                None => gone.strong = true,
            },
            '*' if !two && !gone.emph => match find(&c, i + 1, '*') {
                Some(j) if j > i + 1 => {
                    flush(&mut out, &mut buf);
                    out.push(Inline::Emph(text_of(&c[i + 1..j])));
                    i = j + 1;
                    continue;
                }
                Some(_) => {}
                None => gone.emph = true,
            },
            '[' => {
                if let Some((text, url, end)) = link_at(&c, i, &mut gone, &mut next_close) {
                    flush(&mut out, &mut buf);
                    out.push(Inline::Link { text, url });
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        buf.push(c[i]);
        i += 1;
    }
    flush(&mut out, &mut buf);
    out
}

/// Paragraph lines: parsed line by line, with `\n` between them.
fn inlines(lines: &[&str]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let mut part = inline_line(line);
        if n > 0 {
            part.insert(0, Inline::Text("\n".to_string()));
        }
        for item in part {
            match (out.last_mut(), item) {
                (Some(Inline::Text(a)), Inline::Text(b)) => a.push_str(&b),
                (_, item) => out.push(item),
            }
        }
    }
    out
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let rest = line.get(hashes..)?;
    ((1..=6).contains(&hashes) && rest.starts_with(' ')).then(|| (hashes as u8, rest.trim()))
}

/// List item: (numbered, text).
fn list_item(line: &str) -> Option<(bool, &str)> {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(m) {
            return Some((false, rest.trim()));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let rest = line.get(digits..)?.strip_prefix(". ")?;
    (digits > 0).then(|| (true, rest.trim()))
}

fn starts_block(line: &str) -> bool {
    line.trim().is_empty()
        || line.starts_with("```")
        || heading(line).is_some()
        || list_item(line).is_some()
}

pub fn render(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < lines.len() {
        let line = lines[i];
        if let Some(info) = line.strip_prefix("```") {
            let lang = Some(info.trim().to_string()).filter(|l| !l.is_empty());
            let body: Vec<&str> = lines[i + 1..]
                .iter()
                .take_while(|l| !l.starts_with("```"))
                .copied()
                .collect();
            i += body.len() + 2;
            out.push(Block::Code {
                lang,
                text: body.join("\n"),
            });
        } else if let Some((level, text)) = heading(line) {
            out.push(Block::Heading {
                level,
                inlines: inlines(&[text]),
            });
            i += 1;
        } else if let Some((ordered, _)) = list_item(line) {
            let mut items = Vec::new();
            while let Some((o, text)) = lines.get(i).and_then(|l| list_item(l)) {
                if o != ordered {
                    break;
                }
                items.push(inlines(&[text]));
                i += 1;
            }
            out.push(Block::List { ordered, items });
        } else if line.trim().is_empty() {
            i += 1;
        } else {
            let end = (i + 1..lines.len())
                .find(|j| starts_block(lines[*j]))
                .unwrap_or(lines.len());
            out.push(Block::Paragraph(inlines(&lines[i..end])));
            i = end;
        }
    }
    out
}
