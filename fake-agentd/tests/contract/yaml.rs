//! Parsing the YAML subset used by `openapi-draft.yaml`: block maps and lists, strings in single and
//! double quotes, single-line and multi-line `{...}` and `[...]`, block scalars `>` and `|`.
//!
//! This is not a full YAML parser: anchors, tags and multiple documents are not supported. A YAML crate dependency
//! is not allowed by the dependency policy, so the parser is custom; silent parsing errors are guarded against by
//! the `contract_yaml_*` tests (counters are checked against an independent line count of the file).

#[derive(Debug, Clone, PartialEq)]
pub enum Y {
    Null,
    Str(String),
    Map(Vec<(String, Y)>),
    Seq(Vec<Y>),
}

impl Y {
    pub fn get(&self, key: &str) -> Option<&Y> {
        match self {
            Y::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Y> {
        keys.iter().try_fold(self, |y, k| y.get(k))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Y::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> &[Y] {
        match self {
            Y::Seq(s) => s,
            _ => &[],
        }
    }

    pub fn entries(&self) -> &[(String, Y)] {
        match self {
            Y::Map(m) => m,
            _ => &[],
        }
    }

    pub fn strs(&self) -> Vec<&str> {
        self.as_seq().iter().filter_map(Y::as_str).collect()
    }
}

struct Line {
    indent: usize,
    text: String,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].replace("\\\"", "\"");
    }
    s.to_string()
}

/// Position of the first `:` outside quotes followed by a space or end of line.
fn split_key(text: &str) -> Option<(String, String)> {
    let b = text.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &c) in b.iter().enumerate() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') if i == 0 => quote = Some(c),
            (None, b':') if i + 1 == b.len() || b[i + 1] == b' ' => {
                return Some((unquote(&text[..i]), text[i + 1..].trim().to_string()));
            }
            _ => {}
        }
    }
    None
}

fn balance(s: &str) -> i32 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{' | '[') => depth += 1,
            (None, '}' | ']') => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn split_top(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0);
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '{' | '[') => {
                depth += 1;
                cur.push(c);
            }
            (None, '}' | ']') => {
                depth -= 1;
                cur.push(c);
            }
            (None, ',') if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn flow(s: &str) -> Y {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        let entries = split_top(inner)
            .iter()
            .filter_map(|item| split_key(item.trim()))
            .map(|(k, v)| (k, flow(&v)))
            .collect();
        return Y::Map(entries);
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Y::Seq(split_top(inner).iter().map(|i| flow(i)).collect());
    }
    if s.is_empty() || s == "null" || s == "~" {
        return Y::Null;
    }
    Y::Str(unquote(s))
}

struct Parser {
    lines: Vec<Line>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Line> {
        self.lines.get(self.pos)
    }

    /// Value after `key:` or `- `: flow, block scalar, plain scalar with continuation or a nested block.
    fn value(&mut self, rest: &str, indent: usize) -> Result<Y, String> {
        let first = rest.chars().next();
        if matches!(first, Some('>' | '|')) {
            let mut parts = Vec::new();
            while self.peek().is_some_and(|l| l.indent > indent) {
                parts.push(self.lines[self.pos].text.clone());
                self.pos += 1;
            }
            return Ok(Y::Str(parts.join(" ")));
        }
        if matches!(first, Some('{' | '[')) {
            let mut text = rest.to_string();
            while balance(&text) > 0 {
                let Some(l) = self.peek() else {
                    return Err(format!("unclosed bracket: {text}"));
                };
                text.push(' ');
                text.push_str(&l.text);
                self.pos += 1;
            }
            return Ok(flow(&text));
        }
        if first.is_some() {
            let mut text = rest.to_string();
            while self.peek().is_some_and(|l| l.indent > indent) {
                text.push(' ');
                text.push_str(&self.lines[self.pos].text);
                self.pos += 1;
            }
            return Ok(flow(&text));
        }
        match self.peek() {
            Some(l) if l.indent > indent => {
                let child = l.indent;
                self.block(child)
            }
            Some(l) if l.indent == indent && l.text.starts_with("- ") => self.block(indent),
            _ => Ok(Y::Null),
        }
    }

    fn block(&mut self, indent: usize) -> Result<Y, String> {
        if self.peek().is_some_and(|l| l.text.starts_with("- ")) {
            let mut items = Vec::new();
            while let Some(l) = self.peek() {
                if l.indent != indent || !l.text.starts_with("- ") {
                    break;
                }
                let rest = l.text[2..].trim().to_string();
                if split_key(&rest).is_some() && !rest.starts_with(['{', '[']) {
                    self.lines[self.pos] = Line {
                        indent: indent + 2,
                        text: rest,
                    };
                    items.push(self.block(indent + 2)?);
                } else {
                    self.pos += 1;
                    items.push(self.value(&rest, indent)?);
                }
            }
            return Ok(Y::Seq(items));
        }
        let mut entries = Vec::new();
        while let Some(l) = self.peek() {
            if l.indent != indent {
                break;
            }
            let text = l.text.clone();
            let Some((key, rest)) = split_key(&text) else {
                return Err(format!("expected `key: value`: {text}"));
            };
            self.pos += 1;
            entries.push((key, self.value(&rest, indent)?));
        }
        Ok(Y::Map(entries))
    }
}

pub fn parse(src: &str) -> Result<Y, String> {
    let lines = src
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| Line {
            indent: l.len() - l.trim_start().len(),
            text: l.trim().to_string(),
        })
        .collect();
    let mut p = Parser { lines, pos: 0 };
    let y = p.block(0)?;
    match p.peek() {
        None => Ok(y),
        Some(l) => Err(format!("parsing stopped at line: {}", l.text)),
    }
}
