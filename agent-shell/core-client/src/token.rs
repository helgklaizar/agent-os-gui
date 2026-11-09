//! Token source and the token itself: the GUI token lives only in memory.

use std::fmt;
#[cfg(feature = "test-token-file")]
use std::path::PathBuf;

/// Where the client gets the token from.
#[derive(Debug, Clone)]
pub enum TokenSource {
    /// Product: a descriptor inherited from `agentd` (api.md section 1.1); read once until the end.
    Fd(i32),
    /// Tests and `fake-agentd` only (`AGENTD_TOKEN_FILE`); not compiled without the `test-token-file` feature.
    #[cfg(feature = "test-token-file")]
    TestFile(PathBuf),
    /// Token already in memory (after `issue_token` in tests and for CLI calls).
    Value(Token),
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("failed to read the token: {0}")]
    Read(String),
    #[error("token is empty or contains invalid characters")]
    Invalid,
}

/// The token value. `Debug` does not print it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Token {
        Token(value.into())
    }

    /// Value for the `Authorization` header; it is not handed out anywhere else.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<hidden>)")
    }
}

impl TokenSource {
    /// Test source from `AGENTD_TOKEN_FILE` (the caller passes the variable's value); an empty value gives `None`.
    #[cfg(feature = "test-token-file")]
    pub fn from_test_env_value(path: Option<&str>) -> Option<TokenSource> {
        path.filter(|p| !p.is_empty())
            .map(|p| TokenSource::TestFile(PathBuf::from(p)))
    }

    /// Reads the token. For `Fd` reads to end of file of the descriptor and blocks the thread until the writer closes it.
    pub fn read(&self) -> Result<Token, TokenError> {
        match self {
            TokenSource::Fd(fd) => {
                let dir = if cfg!(target_os = "linux") {
                    "/proc/self/fd"
                } else {
                    "/dev/fd"
                };
                read_token_file(&std::path::Path::new(dir).join(fd.to_string()))
            }
            #[cfg(feature = "test-token-file")]
            TokenSource::TestFile(path) => read_token_file(path),
            TokenSource::Value(t) => Ok(t.clone()),
        }
    }
}

/// The descriptor is read as the file `/proc/self/fd/N`: `unsafe` is forbidden in `core-client` (`forbid`), and taking an fd via
/// `from_raw_fd` is impossible without it. Fine for pipes, memfd and files; for sockets `ui` passes the token differently.
fn read_token_file(path: &std::path::Path) -> Result<Token, TokenError> {
    use std::io::Read;
    let file =
        std::fs::File::open(path).map_err(|e| TokenError::Read(format!("{:?}", e.kind())))?;
    let mut buf = Vec::new();
    file.take(MAX_TOKEN_LEN + 1)
        .read_to_end(&mut buf)
        .map_err(|e| TokenError::Read(format!("{:?}", e.kind())))?;
    let text = String::from_utf8(buf).map_err(|_| TokenError::Invalid)?;
    let text = text.trim();
    let visible = text.bytes().all(|b| b.is_ascii_graphic());
    if text.is_empty() || text.len() as u64 > MAX_TOKEN_LEN || !visible {
        return Err(TokenError::Invalid);
    }
    Ok(Token::new(text))
}

const MAX_TOKEN_LEN: u64 = 512;
