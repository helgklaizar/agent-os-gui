//! Files `AGENT.md` and `memory/` (`docs/reference/api.md` 5.7): path parsing and the store port. Only these two roots are opened;
//! any other path, `..`, absolute paths, backslashes and NUL are rejected before touching the disk.

use serde_json::{Value, json};

use crate::PortFuture;

/// File content limit on write and read, bytes.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_PATH_BYTES: usize = 512;
const MAX_COMPONENT_BYTES: usize = 255;
const EXTENSIONS: [&str; 2] = ["md", "txt"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Prompt,
    Memory,
}

impl FileKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Prompt => "prompt",
            FileKind::Memory => "memory",
        }
    }
}

/// Normalized relative path inside `~/.agentos`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePath {
    pub rel: String,
    pub kind: FileKind,
}

/// `AGENT.md` or `memory/<…>.md|txt` without `..`, `.`, empty parts, absolute paths and special characters; case matters.
#[must_use]
pub fn normalize(path: &str) -> Option<FilePath> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.contains(['\0', '\\'])
        || path.chars().any(char::is_control)
    {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.len() > MAX_COMPONENT_BYTES)
    {
        return None;
    }
    if parts == ["AGENT.md"] {
        return Some(FilePath {
            rel: path.to_string(),
            kind: FileKind::Prompt,
        });
    }
    let last = parts.last()?;
    let ext_ok = last.rsplit_once('.').is_some_and(|(stem, e)| {
        !stem.is_empty() && EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
    });
    (parts.len() >= 2 && parts[0] == "memory" && ext_ok).then(|| FilePath {
        rel: path.to_string(),
        kind: FileKind::Memory,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    /// Content hash: also the `ETag`.
    pub rev: String,
}

impl FileInfo {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"path": self.path, "kind": self.kind.as_str(), "size": self.size, "rev": self.rev})
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileError {
    /// Path outside the roots, a link, an unsuitable name.
    Denied,
    NotFound,
    /// `If-Match` did not match; the value is the current `rev` (`0` — no file).
    Stale(String),
    TooLarge,
    Unavailable,
}

/// `rev` of a missing file: this is how a client creates a new file (`If-Match: "0"`).
pub const MISSING_REV: &str = "0";

/// File store (implemented by `agentd-db`: the file system under `~/.agentos`).
pub trait FileStore: Send + Sync + 'static {
    fn list(&self) -> PortFuture<'_, Result<Vec<FileInfo>, FileError>>;
    fn read(&self, path: FilePath) -> PortFuture<'_, Result<(Vec<u8>, FileInfo), FileError>>;
    /// Write the whole file if `if_match` matches the current `rev`; atomic, `0600`.
    fn write(
        &self,
        path: FilePath,
        if_match: String,
        body: Vec<u8>,
    ) -> PortFuture<'_, Result<FileInfo, FileError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_paths() {
        assert_eq!(normalize("AGENT.md").unwrap().kind, FileKind::Prompt);
        assert_eq!(normalize("memory/a.md").unwrap().kind, FileKind::Memory);
        assert!(normalize("memory/sub/café.TXT").is_some());
    }

    #[test]
    fn everything_else_is_denied() {
        for bad in [
            "",
            "/",
            "/etc/passwd",
            "..",
            "../AGENT.md",
            "memory/../keys.env",
            "memory/./a.md",
            "memory//a.md",
            "memory/",
            "memory",
            "memory/a",
            "memory/.md",
            "memory/a.exe",
            "keys.env",
            "agent.md",
            "Memory/a.md",
            "AGENT.md/x",
            "AGENT.md/..",
            "sessions.json",
            ".agentos/AGENT.md",
            "memory\\a.md",
            "memory/a\0.md",
            "memory/a\n.md",
            "C:/a.md",
        ] {
            assert!(normalize(bad).is_none(), "{bad:?}");
        }
        assert!(normalize(&format!("memory/{}.md", "é".repeat(300))).is_none());
    }
}
