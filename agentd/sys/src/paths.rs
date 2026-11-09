//! Filesystem-level checks of user paths.

use std::path::{Component, Path, PathBuf};

/// Whether a path escapes the home directory through a symlink.
///
/// Only the existing part of the path is checked: if one of its components is a symlink and the result
/// (`canonicalize` of the longest existing prefix) lies outside `canonicalize(home)`, the path "escapes". A path without
/// symlinks, including one outside home or nonexistent, does not escape: this is a working rule, not a sandbox.
#[must_use]
pub fn symlink_escapes_home(path: &Path, home: &Path) -> bool {
    let Ok(home) = home.canonicalize() else {
        return false;
    };
    let mut current = PathBuf::new();
    let mut through_symlink = false;
    for c in path.components() {
        current.push(c);
        if matches!(c, Component::RootDir | Component::Prefix(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(m) => through_symlink |= m.file_type().is_symlink(),
            Err(_) => break,
        }
    }
    if !through_symlink {
        return false;
    }
    // Longest existing prefix: the result of all symlinks on the path.
    let mut existing = path.to_path_buf();
    while !existing.exists() {
        if !existing.pop() {
            return false;
        }
    }
    existing
        .canonicalize()
        .is_ok_and(|real| !real.starts_with(&home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_missing_paths_do_not_escape() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        std::fs::create_dir_all(home.join("a")).unwrap();
        assert!(!symlink_escapes_home(&home.join("a"), &home));
        assert!(!symlink_escapes_home(&home.join("missing/x"), &home));
        assert!(!symlink_escapes_home(Path::new("/nonexistent-x/y"), &home));
    }

    #[test]
    fn link_out_of_home_escapes_link_inside_does_not() {
        let d = tempfile::tempdir().unwrap();
        let (home, out) = (d.path().join("home"), d.path().join("out"));
        std::fs::create_dir_all(home.join("real")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::os::unix::fs::symlink(&out, home.join("bad")).unwrap();
        std::os::unix::fs::symlink(home.join("real"), home.join("good")).unwrap();
        assert!(symlink_escapes_home(&home.join("bad"), &home));
        assert!(symlink_escapes_home(&home.join("bad/sub/not-yet"), &home));
        assert!(!symlink_escapes_home(&home.join("good"), &home));
    }
}
