//! A7-09: `agentd backup` is a consistent snapshot of `sessions.db` (`VACUUM INTO`, the writer is not blocked), `index.db`, `AGENT.md`
//! and `memory/` in one `tar` archive encrypted with `age` for recipients; without recipients no archive is created; no secrets.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use agentd_db::{Db, DbConfig};

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("agentos");
    let db = root.join("sessions.db");
    {
        let d = Arc::new(Db::open(DbConfig::new(&db)).unwrap());
        d.write(|tx| {
            tx.execute("INSERT INTO sessions(name, created_at, updated_at) VALUES ('Backup session', 1, 1)", [])?;
            Ok(())
        })
        .await
        .unwrap();
    }
    std::fs::create_dir_all(root.join("memory")).unwrap();
    std::fs::write(root.join("AGENT.md"), "Prompt").unwrap();
    std::fs::write(root.join("memory/a.md"), "Fact").unwrap();
    std::fs::write(root.join("keys.env"), "KEY=not-for-archive").unwrap();
    std::fs::write(root.join("index.db"), "INDEX").unwrap();
    Rig {
        _dir: dir,
        root,
        db,
    }
}

fn backup(r: &Rig, recipients: &[&str]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.args(["backup", "--db"])
        .arg(&r.db)
        .arg("--root")
        .arg(&r.root);
    for x in recipients {
        c.args(["--recipient", x]);
    }
    c.env_remove("AGENTD_DB").output().unwrap()
}

/// ustar parsing: name -> content.
fn untar(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 512 <= data.len() && data[i] != 0 {
        let name = String::from_utf8_lossy(&data[i..i + 100])
            .trim_end_matches('\0')
            .to_string();
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&data[i + 124..i + 135]).trim_matches(['\0', ' ']),
            8,
        )
        .unwrap();
        let sum: u32 = data[i..i + 148]
            .iter()
            .chain(&data[i + 156..i + 512])
            .map(|b| u32::from(*b))
            .sum::<u32>()
            + 8 * 32;
        let want = u32::from_str_radix(
            String::from_utf8_lossy(&data[i + 148..i + 154]).trim_matches(['\0', ' ']),
            8,
        )
        .unwrap();
        assert_eq!(sum, want, "header checksum of {name}");
        out.push((name, data[i + 512..i + 512 + size].to_vec()));
        i += 512 + size.div_ceil(512) * 512;
    }
    out
}

fn decrypt(path: &Path, id: &age::x25519::Identity) -> Vec<u8> {
    let d = age::Decryptor::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = Vec::new();
    d.decrypt(std::iter::once(id as &dyn age::Identity))
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_is_encrypted_consistent_and_without_secrets() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let o = backup(&r, &[&id.to_public().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let path = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
    assert!(
        path.starts_with(r.root.join("export")) && path.to_string_lossy().ends_with(".tar.age"),
        "{path:?}"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let raw = std::fs::read(&path).unwrap();
    assert!(
        !String::from_utf8_lossy(&raw).contains("Backup session"),
        "no plaintext in the file"
    );
    let files = untar(&decrypt(&path, &id));
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        names,
        ["sessions.db", "index.db", "AGENT.md", "memory/a.md"],
        "keys.env is not in the archive"
    );
    let snap = files.iter().find(|f| f.0 == "sessions.db").unwrap();
    let check = tempfile::tempdir().unwrap();
    std::fs::write(check.path().join("restored.db"), &snap.1).unwrap();
    let c = agentd_db::Connection::open(check.path().join("restored.db")).unwrap();
    let ic: String = c
        .query_row("PRAGMA integrity_check", [], |x| x.get(0))
        .unwrap();
    assert_eq!(ic, "ok");
    let name: String = c
        .query_row("SELECT name FROM sessions", [], |x| x.get(0))
        .unwrap();
    assert_eq!(name, "Backup session");
    assert_eq!(files[2].1, "Prompt".as_bytes());
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_needs_valid_recipients_and_leaves_nothing() {
    let r = rig().await;
    for args in [vec![], vec!["not-a-key"]] {
        let o = backup(&r, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(o.stdout.is_empty());
    }
    assert!(
        !r.root.join("export").exists()
            || std::fs::read_dir(r.root.join("export"))
                .unwrap()
                .next()
                .is_none()
    );
}

/// Writes the secret part of an age key to a `0600` file (`age-keygen` format) and returns the path.
fn identity_file(dir: &Path, id: &age::x25519::Identity) -> PathBuf {
    use age::secrecy::ExposeSecret;
    let p = dir.join("identity.txt");
    std::fs::write(&p, format!("{}\n", id.to_string().expose_secret())).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    p
}

/// Restore is isolated from the user's live `agentd`: the socket is looked up in an empty directory next to the target, not in
/// the VM's `$XDG_RUNTIME_DIR` (the real core listens there and `restore` rightly refuses).
fn restore(archive: &Path, identity: &Path, db: &Path, root: &Path) -> std::process::Output {
    let isolated = root.parent().expect("restore root inside the directory");
    Command::new(env!("CARGO_BIN_EXE_agentd"))
        .args(["restore", "--archive"])
        .arg(archive)
        .arg("--identity")
        .arg(identity)
        .arg("--db")
        .arg(db)
        .arg("--root")
        .arg(root)
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env("XDG_RUNTIME_DIR", isolated)
        .output()
        .unwrap()
}

fn make_backup(r: &Rig, id: &age::x25519::Identity) -> PathBuf {
    let o = backup(r, &[&id.to_public().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())
}

fn integrity_ok(db: &Path) {
    let c = agentd_db::Connection::open(db).unwrap();
    let ic: String = c
        .query_row("PRAGMA integrity_check", [], |x| x.get(0))
        .unwrap();
    assert_eq!(ic, "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_roundtrip_restore() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let target = tempfile::tempdir().unwrap();
    let root = target.path().join("agentos");
    let db = root.join("sessions.db");
    let key = identity_file(target.path(), &id);
    let o = restore(&archive, &key, &db, &root);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    integrity_ok(&db);
    let c = agentd_db::Connection::open(&db).unwrap();
    let name: String = c
        .query_row("SELECT name FROM sessions", [], |x| x.get(0))
        .unwrap();
    assert_eq!(name, "Backup session");
    assert_eq!(
        std::fs::read(root.join("AGENT.md")).unwrap(),
        "Prompt".as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("memory/a.md")).unwrap(),
        "Fact".as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("index.db")).unwrap(),
        "INDEX".as_bytes()
    );
    assert!(
        !root.join("keys.env").exists(),
        "there were no secrets in the archive"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_moves_wal_shm_to_bak() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let target = tempfile::tempdir().unwrap();
    let root = target.path().join("agentos");
    std::fs::create_dir_all(&root).unwrap();
    let db = root.join("sessions.db");
    for (n, body) in [
        ("sessions.db", "OLD-DB"),
        ("sessions.db-wal", "OLD-WAL"),
        ("sessions.db-shm", "OLD-SHM"),
        ("AGENT.md", "OLD-PROMPT"),
    ] {
        std::fs::write(root.join(n), body).unwrap();
    }
    let key = identity_file(target.path(), &id);
    let o = restore(&archive, &key, &db, &root);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        !root.join("sessions.db-wal").exists(),
        "-wal did not stay in place"
    );
    assert!(
        !root.join("sessions.db-shm").exists(),
        "-shm did not stay in place"
    );
    let moved = |prefix: &str| -> Vec<(String, Vec<u8>)> {
        std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with(prefix)
                    .then(|| (n, std::fs::read(e.path()).unwrap()))
            })
            .collect()
    };
    for (prefix, body) in [
        ("sessions.db-wal.bak-", "OLD-WAL"),
        ("sessions.db-shm.bak-", "OLD-SHM"),
        ("sessions.db.bak-", "OLD-DB"),
        ("AGENT.md.bak-", "OLD-PROMPT"),
    ] {
        let m = moved(prefix);
        assert_eq!(m.len(), 1, "{prefix}: {m:?}");
        assert_eq!(m[0].1, body.as_bytes(), "{prefix}: content not lost");
        let stamp = m[0].0.rsplit(".bak-").next().unwrap();
        assert!(
            stamp.len() == 8 && stamp.bytes().all(|b| b.is_ascii_digit()),
            "{}",
            m[0].0
        );
    }
    integrity_ok(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_with_wrong_key_changes_nothing() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let target = tempfile::tempdir().unwrap();
    let root = target.path().join("agentos");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("sessions.db"), "LIVE-DB").unwrap();
    let other = age::x25519::Identity::generate();
    let key = identity_file(target.path(), &other);
    let o = restore(&archive, &key, &root.join("sessions.db"), &root);
    assert!(!o.status.success(), "foreign key does not open the archive");
    assert_eq!(
        std::fs::read(root.join("sessions.db")).unwrap(),
        "LIVE-DB".as_bytes()
    );
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["sessions.db"], "nothing moved or created");
}
