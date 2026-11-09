//! Parsing of a `text/event-stream` (SSE) stream.

/// One stream event: name (`event:`) and data (`data:`, lines joined by `\n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Accumulates bytes and yields complete events. Lines are split on bytes, so multibyte characters
/// split across chunk boundaries stay intact.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
}

impl SseParser {
    #[must_use]
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// Appends a chunk of bytes; returns the events closed by a blank line.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw[..pos]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if self.has_data {
                    out.push(SseEvent {
                        event: self.event.take(),
                        data: self.data.join("\n"),
                    });
                }
                self.event = None;
                self.data.clear();
                self.has_data = false;
            } else if !line.starts_with(':') {
                let (name, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match name {
                    "event" => self.event = Some(value.to_string()),
                    "data" => {
                        self.data.push(value.to_string());
                        self.has_data = true;
                    }
                    _ => {}
                }
            }
        }
        out
    }
}
