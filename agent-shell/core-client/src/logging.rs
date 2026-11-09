//! Core client and `agent-shell` log: `tracing` to stderr (collected by journald),
//! level from `RUST_LOG` (default `info`), every record passes through secret masking.
//!
//! Masking is a second line of defence: the code never writes tokens or user text to the log at all, and [`scrub`] guarantees
//! that an accidentally included value (header, WS subprotocol, query parameter, token in the generator's format) is replaced
//! with `<redacted>` before output, including output from third-party crates at `trace` level.

use std::io::{self, Write};

use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

pub const REDACTED: &str = "<redacted>";

/// Words (lowercase) after which a secret value follows. The value ends at a space, quote, `&`, `,`, `;`, `}`, `)`.
const MARKERS: [&str; 10] = [
    "bearer ",
    "bearer.",
    "authorization=",
    "authorization:",
    "token=",
    "secret=",
    "secret_ref=",
    "password=",
    "api_key=",
    "apikey=",
];

/// Start of a token in the core generator's format (`agt_` and base64url).
const TOKEN_PREFIX: &str = "agt_";

fn ends_value(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '&' | ',' | ';' | '}' | ')')
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// Replaces the value after each `marker` (case-insensitive). Already masked and empty values are left alone.
fn mask_after(s: &str, marker: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(rel) = lower[at..].find(marker) {
        let mut start = at + rel + marker.len();
        out.push_str(&s[at..start]);
        // Spaces and an opening quote between the word and the value stay in place.
        while let Some(c) = s[start..].chars().next().filter(|c| *c == ' ' || *c == '"') {
            out.push(c);
            start += c.len_utf8();
        }
        let rest = &s[start..];
        let len = rest.find(ends_value).unwrap_or(rest.len());
        if len > 0 && !rest.starts_with(REDACTED) {
            out.push_str(REDACTED);
            at = start + len;
        } else {
            at = start;
        }
    }
    out.push_str(&s[at..]);
    out
}

/// Replaces tokens in the generator's format (`agt_…`) wherever they appear.
fn mask_generated(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(rel) = s[at..].find(TOKEN_PREFIX) {
        let start = at + rel;
        let body = &s[start + TOKEN_PREFIX.len()..];
        let len = body.find(|c: char| !is_token_char(c)).unwrap_or(body.len());
        out.push_str(&s[at..start]);
        if len > 0 {
            out.push_str(REDACTED);
        } else {
            out.push_str(TOKEN_PREFIX);
        }
        at = start + TOKEN_PREFIX.len() + len;
    }
    out.push_str(&s[at..]);
    out
}

/// Replaces secret values in a log line with [`REDACTED`].
pub fn scrub(line: &str) -> String {
    let marked = MARKERS
        .iter()
        .fold(line.to_string(), |s, m| mask_after(&s, m));
    mask_generated(&marked)
}

/// A writer that masks every chunk of output. `fmt` writes a record in one `write_all` call.
pub struct ScrubWriter<W: Write>(W);

impl<W: Write> Write for ScrubWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(scrub(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// A writer factory with masking on top of any other factory.
pub struct ScrubMake<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for ScrubMake<M> {
    type Writer = ScrubWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        ScrubWriter(self.0.make_writer())
    }
}

/// Subscriber with the product's format and masking; `make` is where to write (stderr in the product, a buffer in tests).
pub fn subscriber<M>(make: M, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    M: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(ScrubMake(make))
        .with_ansi(false)
        .finish()
}

/// Filter from `RUST_LOG`, otherwise `info`.
pub fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Installs the product's global subscriber (stderr). `false` if a subscriber is already installed.
pub fn init() -> bool {
    tracing::subscriber::set_global_default(subscriber(io::stderr, env_filter())).is_ok()
}
