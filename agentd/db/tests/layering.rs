//! A1-03: the rule "only `db` writes to SQLite" (docs/architecture.md section 4): `rusqlite` is declared only in `agentd-db`.

use std::path::Path;

fn declares_rusqlite(manifest: &str) -> bool {
    manifest
        .lines()
        .any(|l| l.trim_start().starts_with("rusqlite"))
}

#[test]
fn declares_rusqlite_detector_catches_bad_sample() {
    assert!(declares_rusqlite(
        "[dependencies]\nrusqlite.workspace = true\n"
    ));
    assert!(declares_rusqlite("rusqlite = { workspace = true }"));
    assert!(!declares_rusqlite(
        "[dependencies]\nserde.workspace = true\n"
    ));
}

#[test]
fn rusqlite_declared_only_in_db_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap().map(Result::unwrap) {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() || entry.file_name() == "db" {
            continue;
        }
        if declares_rusqlite(&std::fs::read_to_string(&manifest).unwrap()) {
            offenders.push(manifest);
        }
    }
    assert!(
        offenders.is_empty(),
        "rusqlite outside agentd-db: {offenders:?}"
    );
}
