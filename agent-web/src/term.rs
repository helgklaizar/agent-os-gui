//! Agent terminal (view only): session `terminal.*` frames to lines with SGR style.
//! xterm.js is not used: it inserts a `<style>` element and the page CSP forbids inline styles. ANSI is parsed
//! here, colors are CSS classes; everything else from control sequences is dropped.

use serde_json::Value;

/// Limit of accumulated output (docs/reference/api.md: 256 KiB); older output is dropped and a marker is set.
pub const LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<u8>,
    pub bold: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub stderr: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Term {
    pub spans: Vec<Span>,
    pub truncated: bool,
    size: usize,
    style: Style,
}

fn apply_sgr(style: &mut Style, params: &str) {
    let mut any = false;
    for p in params.split(';') {
        any = true;
        match p.parse::<u16>().unwrap_or(0) {
            0 => *style = Style::default(),
            1 => style.bold = true,
            4 => style.underline = true,
            22 => style.bold = false,
            24 => style.underline = false,
            n @ 30..=37 => style.fg = Some(u8::try_from(n - 30).unwrap_or(0)),
            n @ 90..=97 => style.fg = Some(u8::try_from(n - 90 + 8).unwrap_or(0)),
            39 => style.fg = None,
            _ => {}
        }
    }
    if !any {
        *style = Style::default();
    }
}

/// Parse a chunk: text, `\n`, `\t` are kept, SGR changes the style, other CSI/OSC and control characters are dropped.
pub fn parse_ansi(input: &str, style: &mut Style, stderr: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Span>, style: Style| {
        if !buf.is_empty() {
            out.push(Span {
                text: std::mem::take(buf),
                style,
                stderr,
            });
        }
    };
    let mut it = input.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.peek() {
                Some('[') => {
                    it.next();
                    let mut params = String::new();
                    let mut fin = None;
                    for d in it.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&d) {
                            fin = Some(d);
                            break;
                        }
                        params.push(d);
                    }
                    if fin == Some('m') {
                        flush(&mut buf, &mut out, *style);
                        apply_sgr(style, &params);
                    }
                }
                Some(']') => {
                    // OSC: up to BEL or ESC \.
                    it.next();
                    while let Some(d) = it.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\n' | '\t' => buf.push(c),
            c if c.is_control() => {}
            c => buf.push(c),
        }
    }
    flush(&mut buf, &mut out, *style);
    out
}

impl Term {
    fn push_spans(&mut self, spans: Vec<Span>) {
        for s in spans {
            self.size += s.text.len();
            self.spans.push(s);
        }
        while self.size > LIMIT && !self.spans.is_empty() {
            let first = self.spans.remove(0);
            self.size -= first.text.len();
            self.truncated = true;
        }
    }

    /// Session frame: `terminal.command`, `terminal.output`, `terminal.status`; anything else returns `false`.
    pub fn on_frame(&mut self, data: &Value) -> bool {
        let Some(kind) = data.get("kind").and_then(Value::as_str) else {
            return false;
        };
        match kind {
            "terminal.command" => {
                let cmd = data
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut st = Style {
                    fg: Some(6),
                    bold: true,
                    underline: false,
                };
                let spans = parse_ansi(&format!("$ {cmd}\n"), &mut st, false);
                self.style = Style::default();
                self.push_spans(spans);
                true
            }
            "terminal.output" => {
                let text = data.get("data").and_then(Value::as_str).unwrap_or_default();
                let stderr = data.get("stream").and_then(Value::as_str) == Some("stderr");
                let mut st = self.style;
                let spans = parse_ansi(text, &mut st, stderr);
                self.style = st;
                self.push_spans(spans);
                true
            }
            "terminal.status" => {
                let status = data.get("status").and_then(Value::as_str).unwrap_or("?");
                let code = data
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .map(|c| format!(" (code {c})"))
                    .unwrap_or_default();
                let mut st = Style {
                    fg: Some(8),
                    bold: false,
                    underline: false,
                };
                let spans = parse_ansi(&format!("[{status}{code}]\n"), &mut st, false);
                self.style = Style::default();
                self.push_spans(spans);
                true
            }
            _ => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(s: &str) -> Vec<Span> {
        parse_ansi(s, &mut Style::default(), false)
    }

    #[test]
    fn sgr_colors_and_reset() {
        let v = run("a\u{1b}[31mred\u{1b}[0m b\u{1b}[1;92mx\u{1b}[22m");
        assert_eq!(v[0].text, "a");
        assert_eq!((v[1].text.as_str(), v[1].style.fg), ("red", Some(1)));
        assert_eq!((v[2].text.as_str(), v[2].style), (" b", Style::default()));
        assert_eq!((v[3].style.fg, v[3].style.bold), (Some(10), true));
        assert_eq!(
            parse_ansi(
                "\u{1b}[mz",
                &mut Style {
                    fg: Some(1),
                    bold: true,
                    underline: true
                },
                false
            )[0]
            .style,
            Style::default()
        );
    }

    #[test]
    fn other_sequences_and_controls_are_dropped() {
        assert_eq!(run("a\u{1b}[2J\u{1b}[10;5Hb")[0].text, "ab");
        assert_eq!(run("x\u{1b}]0;title\u{7}y")[0].text, "xy");
        assert_eq!(run("x\u{1b}]0;t\u{1b}\\y")[0].text, "xy");
        assert_eq!(run("p\u{7}q\r\n\tr")[0].text, "pq\n\tr");
        assert_eq!(run("\u{1b}c!")[0].text, "!");
    }

    #[test]
    fn html_stays_literal_text() {
        assert_eq!(
            run("<img src=x onerror=1>")[0].text,
            "<img src=x onerror=1>"
        );
    }

    #[test]
    fn style_carries_between_chunks_and_frames() {
        let mut t = Term::default();
        assert!(t.on_frame(
            &json!({"kind": "terminal.output", "stream": "stdout", "data": "\u{1b}[32mok"})
        ));
        assert!(
            t.on_frame(&json!({"kind": "terminal.output", "stream": "stderr", "data": " more"}))
        );
        assert_eq!(t.spans[1].style.fg, Some(2));
        assert!(t.spans[1].stderr && !t.spans[0].stderr);
        assert!(!t.on_frame(&json!({"kind": "message.done"})));
        assert!(!t.on_frame(&json!({"x": 1})));
    }

    #[test]
    fn command_and_status_frames() {
        let mut t = Term::default();
        t.on_frame(&json!({"kind": "terminal.command", "command": "ls -la"}));
        t.on_frame(&json!({"kind": "terminal.status", "status": "exited", "exit_code": 2}));
        let all: String = t.spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(all, "$ ls -la\n[exited (code 2)]\n");
        assert!(!t.is_empty());
    }

    #[test]
    fn attributes_switch_on_and_off() {
        let v = run("\u{1b}[4;1;31ma\u{1b}[24mb\u{1b}[22mc\u{1b}[39md");
        assert_eq!(
            (v[0].style.underline, v[0].style.bold, v[0].style.fg),
            (true, true, Some(1))
        );
        assert_eq!((v[1].style.underline, v[1].style.bold), (false, true));
        assert_eq!((v[2].style.bold, v[2].style.fg), (false, Some(1)));
        assert_eq!(v[3].style.fg, None);
    }

    #[test]
    fn exact_limit_is_not_truncated() {
        let mut t = Term::default();
        assert!(t.is_empty());
        let chunk = "x".repeat(LIMIT);
        t.on_frame(&json!({"kind": "terminal.output", "stream": "stdout", "data": chunk}));
        assert!(!t.truncated);
        assert!(!t.is_empty());
        t.on_frame(&json!({"kind": "terminal.output", "stream": "stdout", "data": "y"}));
        assert!(t.truncated);
    }

    #[test]
    fn output_is_bounded() {
        let mut t = Term::default();
        let chunk = "x".repeat(100 * 1024);
        for _ in 0..4 {
            t.on_frame(&json!({"kind": "terminal.output", "stream": "stdout", "data": chunk}));
        }
        assert!(t.truncated);
        let total: usize = t.spans.iter().map(|s| s.text.len()).sum();
        assert!(total <= LIMIT, "{total}");
        assert_eq!(total, 2 * 100 * 1024);
    }
}
