//! The schema lives in one place, `agentd/migrations/`; the mock has no byte-for-byte copies.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sql_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".sql"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Mock files that match a same-named file of `agentd/migrations/` byte for byte.
fn byte_equal_copies() -> Vec<String> {
    let fake = root().join("fake-agentd/migrations");
    let real = root().join("agentd/migrations");
    sql_files(&fake)
        .into_iter()
        .filter(
            |n| match (std::fs::read(fake.join(n)), std::fs::read(real.join(n))) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
        )
        .collect()
}

#[test]
fn fake_agentd_has_no_byte_copies_of_agentd_migrations() {
    let copies = byte_equal_copies();
    assert!(
        copies.is_empty(),
        "copies of the schema in fake-agentd/migrations: {copies:?}; take the file from agentd/migrations via include_str!"
    );
}

#[test]
fn fake_agentd_migration_runner_uses_agentd_files() {
    let src =
        std::fs::read_to_string(root().join("fake-agentd/src/db/migrate.rs")).unwrap_or_default();
    for name in ["0001_init.sql", "0002_bar.sql", "0006_tasks.sql"] {
        assert!(
            src.contains(&format!("agentd/migrations/{name}")),
            "the mock runner does not take {name} from agentd/migrations"
        );
    }
}
