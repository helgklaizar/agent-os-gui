//! A5-fix Z-3: every test that launches the `agentd` binary gets it via `common::agentd_bin` (which rebuilds the binary
//! once per process); a hand-built `target/<profile>/agentd` path yields a stale binary and false-green probes.

use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn only_shared_helper_locates_agentd_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let needle = ".join(\"agentd\")";
    let offenders: Vec<String> = files
        .iter()
        .filter(|p| !p.ends_with("common/bin.rs") && !p.ends_with("bin_fresh.rs"))
        .filter(|p| std::fs::read_to_string(p).unwrap().contains(needle))
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "own agentd path without a rebuild: {offenders:?}"
    );
}
