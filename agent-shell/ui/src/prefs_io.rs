//! Reading and writing `ui.json`: atomically (a temporary file alongside and `rename`), mode `0700` on the directory and `0600` on the file.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use controllers::prefs::{UiFile, UiPrefs};

/// `$XDG_CONFIG_HOME/agent-shell/ui.json`, otherwise `~/.config/agent-shell/ui.json`.
/// An empty or relative `XDG_CONFIG_HOME` counts as unset (XDG specification).
pub fn default_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|x| Path::new(x).is_absolute()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".config"),
    };
    Some(base.join("agent-shell").join("ui.json"))
}

/// Path from the process environment.
pub fn default_path_from_env() -> Option<PathBuf> {
    default_path(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Reads the file: no file gives defaults without a notice; a broken, foreign-version or unreadable one gives
/// defaults and a reason for the log. Startup does not crash on it.
pub fn load(path: &Path) -> (UiFile, Option<String>) {
    match fs::read_to_string(path) {
        Ok(text) => UiFile::parse_or_default(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (UiFile::default(), None),
        Err(e) => (UiFile::default(), Some(format!("ui.json not read: {e}"))),
    }
}

/// Atomic write: a temporary file in the same directory, `fsync`, `rename`.
pub fn save(path: &Path, file: &UiFile) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ui.json path has no parent"))?;
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = dir.join(format!(".ui.json.{}.tmp", std::process::id()));
    let written = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        out.write_all(file.to_json().as_bytes())?;
        out.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        // our own temporary file, not user data; a write error matters more than a cleanup error
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// `ui.json` state in memory and the deferred write.
pub struct PrefsStore {
    path: PathBuf,
    file: UiFile,
    dirty: bool,
}

impl PrefsStore {
    /// Reads the file; the second value is the reason why the defaults were taken.
    pub fn open(path: PathBuf) -> (PrefsStore, Option<String>) {
        let (file, warning) = load(&path);
        (
            PrefsStore {
                path,
                file,
                dirty: false,
            },
            warning,
        )
    }

    pub fn file(&self) -> &UiFile {
        &self.file
    }

    /// Merge a partial update; written on [`flush`](Self::flush).
    pub fn apply(&mut self, prefs: &UiPrefs) {
        let before = self.file;
        self.file.apply(prefs);
        self.dirty |= self.file != before;
    }

    /// Write if anything changed.
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        save(&self.path, &self.file)?;
        self.dirty = false;
        Ok(())
    }
}
