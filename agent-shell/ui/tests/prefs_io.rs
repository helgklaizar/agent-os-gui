//! `ui.json` on disk: path, atomic write, broken file, permissions.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use controllers::prefs::{ColumnPref, ColumnsPrefs, UiFile, UiPrefs, WindowPrefs};
use controllers::{Manual, Theme};
use ui::prefs_io::{PrefsStore, default_path, load, save};

fn sample() -> UiFile {
    UiFile {
        theme: Theme::Dark,
        columns: ColumnsPrefs {
            notifications: ColumnPref {
                manual: Manual::Collapsed,
            },
            sessions: ColumnPref {
                manual: Manual::Auto,
            },
            tools: ColumnPref {
                manual: Manual::Expanded,
            },
        },
        window: WindowPrefs { w: 1440, h: 900 },
        ..UiFile::default()
    }
}

#[test]
fn prefs_path_from_env_values() {
    let expected = |p: &str| Some(PathBuf::from(p).join("agent-shell/ui.json"));
    assert_eq!(
        default_path(Some("/x/cfg"), Some("/home/u")),
        expected("/x/cfg")
    );
    assert_eq!(
        default_path(None, Some("/home/u")),
        expected("/home/u/.config")
    );
    // an empty XDG_CONFIG_HOME counts as unset (XDG specification)
    assert_eq!(
        default_path(Some(""), Some("/home/u")),
        expected("/home/u/.config")
    );
    // a relative XDG path is ignored
    assert_eq!(
        default_path(Some("rel"), Some("/home/u")),
        expected("/home/u/.config")
    );
    assert_eq!(default_path(None, None), None);
    assert_eq!(default_path(None, Some("")), None);
}

#[test]
fn prefs_io_roundtrip_file() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("agent-shell/ui.json");
    save(&path, &sample()).expect("write");
    let (loaded, warning) = load(&path);
    assert_eq!(loaded, sample());
    assert_eq!(warning, None);
}

#[test]
fn prefs_io_missing_file_is_defaults_without_warning() {
    let dir = tempfile::tempdir().expect("directory");
    let (loaded, warning) = load(&dir.path().join("nope/ui.json"));
    assert_eq!(loaded, UiFile::default());
    assert_eq!(warning, None, "a first run is no reason for a warning");
}

#[test]
fn prefs_io_corrupt_falls_back_and_reports() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("ui.json");
    for bad in [
        "{not json",
        "",
        "[1,2]",
        r#"{"version": 99, "theme": "dark"}"#,
    ] {
        fs::write(&path, bad).expect("write");
        let (loaded, warning) = load(&path);
        assert_eq!(loaded, UiFile::default(), "input {bad:?}");
        assert!(
            warning.is_some(),
            "input {bad:?}: a reason for the log is required"
        );
    }
    // after a broken file the next write repairs it
    save(&path, &sample()).expect("write");
    assert_eq!(load(&path).0, sample());
}

#[test]
fn prefs_io_atomic_write_leaves_no_temp_and_replaces_content() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("ui.json");
    fs::write(&path, "old content").expect("write");
    save(&path, &sample()).expect("write");
    let names: Vec<_> = fs::read_dir(dir.path())
        .expect("directory")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, vec!["ui.json"], "no temporary files left");
    assert_eq!(load(&path).0, sample());
}

#[test]
fn prefs_io_creates_private_dir_and_file() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("agent-shell/ui.json");
    save(&path, &sample()).expect("write");
    let mode =
        |p: &std::path::Path| fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode(path.parent().expect("parent")), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn prefs_store_merges_partial_updates_and_flushes_once() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("agent-shell/ui.json");
    let (mut store, warning) = PrefsStore::open(path.clone());
    assert_eq!(warning, None);
    store.flush().expect("empty write");
    assert!(!path.exists(), "the file is not written without changes");

    store.apply(&UiPrefs {
        columns: Some(sample().columns),
        ..UiPrefs::default()
    });
    store.apply(&UiPrefs {
        theme: Some(Theme::Light),
        ..UiPrefs::default()
    });
    assert_eq!(store.file().columns, sample().columns);
    assert_eq!(
        store.file().theme,
        Theme::Light,
        "the theme did not overwrite the columns"
    );
    store.flush().expect("write");
    let (loaded, _) = load(&path);
    assert_eq!(
        (loaded.theme, loaded.columns),
        (Theme::Light, sample().columns)
    );

    let before = fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("time");
    std::thread::sleep(std::time::Duration::from_millis(20));
    store.flush().expect("repeated write");
    let after = fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("time");
    assert_eq!(
        before, after,
        "a repeated flush without changes does not write"
    );
}

#[test]
fn prefs_store_reports_write_failure() {
    let dir = tempfile::tempdir().expect("directory");
    let blocker = dir.path().join("file");
    fs::write(&blocker, "x").expect("write");
    // the parent is a regular file: the directory cannot be created
    let (mut store, _) = PrefsStore::open(blocker.join("ui.json"));
    store.apply(&UiPrefs {
        theme: Some(Theme::Dark),
        ..UiPrefs::default()
    });
    assert!(store.flush().is_err(), "a write error is not swallowed");
}

#[test]
fn prefs_store_open_reads_existing_file() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("ui.json");
    save(&path, &sample()).expect("write");
    let (store, warning) = PrefsStore::open(path);
    assert_eq!(*store.file(), sample());
    assert_eq!(warning, None);
}
