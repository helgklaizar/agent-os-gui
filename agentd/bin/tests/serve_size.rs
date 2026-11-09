//! A5-fix Z-5: `serve.rs` and its modules are no longer than 300 lines.

use std::path::Path;

const LIMIT: usize = 300;

#[test]
fn serve_sources_within_line_limit() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("serve.rs")];
    if let Ok(rd) = std::fs::read_dir(src.join("serve")) {
        files.extend(rd.flatten().map(|e| e.path()));
    }
    for f in files {
        let n = std::fs::read_to_string(&f).unwrap().lines().count();
        assert!(n <= LIMIT, "{}: {n} lines with limit {LIMIT}", f.display());
    }
}
