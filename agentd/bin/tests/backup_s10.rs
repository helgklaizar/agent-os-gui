//! S10-DB-01…05, S10-10: recipient keys from `settings`, second copy, archive verification with an `error` event, restore
//! from an archive and from `pre-migration-VERSION.db`, `user_version`/`foreign_key_check` verification, refusal when `agentd` is live,
//! the "backup -> DB loss -> restore" scenario with matching data and a restore-time measurement.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Instant;

use age::secrecy::ExposeSecret;
use agentd_db::{Connection, Db, DbConfig};

/// Fixture "secret" value: it must not be in the archive (except `memory/`, see the limitation test).
const SECRET: &str = "FIXTURE-SECRET-b7f1c3d9e2a4";

struct Rig {
    _dir: tempfile::TempDir,
    top: PathBuf,
    root: PathBuf,
    db: PathBuf,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let top = dir.path().to_path_buf();
    let root = top.join("agentos");
    let db = root.join("sessions.db");
    {
        let d = Arc::new(Db::open(DbConfig::new(&db)).unwrap());
        d.write(|tx| {
            tx.execute("INSERT INTO sessions(name, created_at, updated_at) VALUES ('Session', 1, 1)", [])?;
            tx.execute("INSERT INTO messages(session_id, role, body_md, ts) VALUES (1, 'user', 'Hello, world', 5)", [])?;
            tx.execute("INSERT INTO messages(session_id, role, body_md, ts) VALUES (1, 'agent', 'Reply', 6)", [])?;
            tx.execute("INSERT INTO settings(key, value_json, rev, updated_at) VALUES ('theme', '\"dark\"', 2, 9)", [])?;
            tx.execute("INSERT INTO events(kind, sub, level, text, unread, ts) VALUES ('msg','sys','info','event',1,7)", [])?;
            Ok(())
        })
        .await
        .unwrap();
    }
    std::fs::create_dir_all(root.join("memory")).unwrap();
    std::fs::write(root.join("AGENT.md"), "Prompt").unwrap();
    std::fs::write(root.join("memory/a.md"), "Fact").unwrap();
    std::fs::write(root.join("keys.env"), format!("API_KEY={SECRET}\n")).unwrap();
    std::fs::write(root.join("index.db"), "INDEX").unwrap();
    Rig {
        _dir: dir,
        top,
        root,
        db,
    }
}

fn agentd() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env_remove("XDG_RUNTIME_DIR");
    c
}

fn backup_cmd(r: &Rig) -> Command {
    let mut c = agentd();
    c.args(["backup", "--db"])
        .arg(&r.db)
        .arg("--root")
        .arg(&r.root);
    c
}

fn restore_cmd(archive: &Path, identity: &Path, db: &Path, root: &Path) -> Command {
    let mut c = agentd();
    c.args(["restore", "--archive"])
        .arg(archive)
        .arg("--identity")
        .arg(identity)
        .arg("--db")
        .arg(db)
        .arg("--root")
        .arg(root);
    c
}

fn verify_cmd(r: &Rig, archive: &Path, identity: &Path) -> Output {
    agentd()
        .args(["backup-verify", "--archive"])
        .arg(archive)
        .arg("--identity")
        .arg(identity)
        .arg("--db")
        .arg(&r.db)
        .output()
        .unwrap()
}

fn identity_file(dir: &Path, name: &str, id: &age::x25519::Identity) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("{}\n", id.to_string().expose_secret())).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    p
}

fn stdout_lines(o: &Output) -> Vec<String> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn make_backup(r: &Rig, id: &age::x25519::Identity) -> PathBuf {
    let o = backup_cmd(r)
        .args(["--recipient", &id.to_public().to_string()])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    PathBuf::from(&stdout_lines(&o)[0])
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

fn set_setting(db: &Path, key: &str, value: &str) {
    let c = Connection::open(db).unwrap();
    c.execute(
        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, ?2, 1, 1)",
        [key, &format!("\"{value}\"")],
    )
    .unwrap();
}

fn error_events(db: &Path) -> Vec<String> {
    let c = Connection::open(db).unwrap();
    let mut s = c
        .prepare("SELECT text FROM events WHERE level = 'error' ORDER BY id")
        .unwrap();
    s.query_map([], |x| x.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn export_files(r: &Rig) -> Vec<String> {
    std::fs::read_dir(r.root.join("export"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Archive with a given `sessions.db`, built bypassing `backup` (which does not let such DBs through): ustar + `age`.
fn forge_archive(r: &Rig, id: &age::x25519::Identity) -> PathBuf {
    use std::io::Write;
    let data = agentd_db::backup_snapshot(&r.db).unwrap();
    let mut h = [0u8; 512];
    h[..11].copy_from_slice(b"sessions.db");
    h[100..108].copy_from_slice(b"0000600\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut tar = h.to_vec();
    tar.extend_from_slice(&data);
    tar.resize(tar.len() + (512 - data.len() % 512) % 512 + 1024, 0);
    let enc =
        age::Encryptor::with_recipients(std::iter::once(&id.to_public() as &dyn age::Recipient))
            .unwrap();
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out).unwrap();
    w.write_all(&tar).unwrap();
    w.finish().unwrap();
    let p = r.top.join("forged.tar.age");
    std::fs::write(&p, out).unwrap();
    p
}

// ---------- S10-DB-01 ----------

#[tokio::test(flavor = "multi_thread")]
async fn backup_takes_both_recipients_from_settings() {
    let r = rig().await;
    let primary = age::x25519::Identity::generate();
    let recovery = age::x25519::Identity::generate();
    set_setting(
        &r.db,
        "backup.recipient.primary",
        &primary.to_public().to_string(),
    );
    set_setting(
        &r.db,
        "backup.recipient.recovery",
        &recovery.to_public().to_string(),
    );
    let o = backup_cmd(&r).output().unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let path = PathBuf::from(&stdout_lines(&o)[0]);
    let a = decrypt(&path, &primary);
    let b = decrypt(&path, &recovery);
    assert_eq!(a, b, "both keys open the same archive");
    assert!(!a.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_from_settings_requires_recovery_recipient() {
    let r = rig().await;
    let primary = age::x25519::Identity::generate();
    set_setting(
        &r.db,
        "backup.recipient.primary",
        &primary.to_public().to_string(),
    );
    let o = backup_cmd(&r).output().unwrap();
    assert!(
        !o.status.success(),
        "no archive without a recovery recipient"
    );
    assert!(stderr(&o).contains("recovery"), "{}", stderr(&o));
    assert!(export_files(&r).is_empty());
    // A value that is not an age key is also a refusal.
    set_setting(&r.db, "backup.recipient.recovery", "not-a-key");
    let o = backup_cmd(&r).output().unwrap();
    assert!(!o.status.success());
    assert!(export_files(&r).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_plaintext_has_no_fixture_secret_but_memory_is_a_known_limit() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let path = make_backup(&r, &id);
    let plain = decrypt(&path, &id);
    let raw = std::fs::read(&path).unwrap();
    let has = |hay: &[u8], needle: &str| hay.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!has(&plain, SECRET), "value from keys.env in the archive");
    assert!(!has(&raw, SECRET));
    // Limitation: a secret that ended up in memory/ is not caught by the value scanner; it is in the archive.
    std::fs::write(r.root.join("memory/b.md"), format!("token {SECRET}")).unwrap();
    let path2 = make_backup(&r, &id);
    assert!(has(&decrypt(&path2, &id), SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_second_copy_on_other_path_is_identical_and_private() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let copy_dir = r.top.join("external");
    let o = backup_cmd(&r)
        .args(["--recipient", &id.to_public().to_string()])
        .arg("--copy-to")
        .arg(&copy_dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let lines = stdout_lines(&o);
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (main, copy) = (PathBuf::from(&lines[0]), PathBuf::from(&lines[1]));
    assert!(copy.starts_with(&copy_dir), "{copy:?}");
    assert_eq!(std::fs::read(&main).unwrap(), std::fs::read(&copy).unwrap());
    assert_eq!(
        std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&copy_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    // The same disk (the temp directory is entirely on one device) does not count as a backup: warning.
    assert!(stderr(&o).contains("same disk"), "{}", stderr(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_refuses_broken_database_reports_error_and_publishes_nothing() {
    let r = rig().await;
    Connection::open(&r.db)
        .unwrap()
        .execute(
            "INSERT INTO messages(session_id, role, body_md, ts) VALUES (999, 'user', 'orphan', 1)",
            [],
        )
        .unwrap();
    let id = age::x25519::Identity::generate();
    let o = backup_cmd(&r)
        .args(["--recipient", &id.to_public().to_string()])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(stderr(&o).contains("foreign_key_check"), "{}", stderr(&o));
    assert!(o.stdout.is_empty());
    assert!(export_files(&r).is_empty(), "{:?}", export_files(&r));
    assert_eq!(error_events(&r.db).len(), 1, "error event in the feed");
}

// ---------- S10-DB-02 ----------

#[tokio::test(flavor = "multi_thread")]
async fn backup_verify_accepts_good_archive_and_stays_silent_in_feed() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let key = identity_file(&r.top, "id.txt", &id);
    let o = verify_cmd(&r, &archive, &key);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(error_events(&r.db).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_verify_flags_corrupted_truncated_and_foreign_archives_with_error_event() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let key = identity_file(&r.top, "id.txt", &id);
    let good = std::fs::read(&archive).unwrap();
    let mut flipped = good.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0xff;
    let truncated = good[..good.len() - 40].to_vec();
    let garbage = "this is not an age archive".as_bytes().to_vec();
    for (i, bad) in [flipped, truncated, garbage].into_iter().enumerate() {
        let p = r.top.join(format!("bad-{i}.tar.age"));
        std::fs::write(&p, bad).unwrap();
        let o = verify_cmd(&r, &p, &key);
        assert!(!o.status.success(), "corrupted archive #{i} accepted");
        assert_eq!(
            error_events(&r.db).len(),
            i + 1,
            "error event for each bad archive"
        );
    }
    let text = error_events(&r.db).join("\n");
    assert!(text.contains("Backup"), "{text}");
    assert!(!text.contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_verify_rejects_archive_with_broken_database() {
    let r = rig().await;
    {
        let c = Connection::open(&r.db).unwrap();
        c.execute(
            "INSERT INTO messages(session_id, role, body_md, ts) VALUES (999, 'user', 'orphan', 1)",
            [],
        )
        .unwrap();
    }
    let id = age::x25519::Identity::generate();
    let archive = forge_archive(&r, &id);
    let key = identity_file(&r.top, "id.txt", &id);
    let o = verify_cmd(&r, &archive, &key);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("foreign_key_check"), "{}", stderr(&o));
    assert_eq!(error_events(&r.db).len(), 1);
}

// ---------- S10-DB-03 / S10-DB-05 ----------

fn target_with_live(top: &Path, body: &str) -> (PathBuf, PathBuf) {
    let root = top.join("restore-target/agentos");
    std::fs::create_dir_all(&root).unwrap();
    let db = root.join("sessions.db");
    std::fs::write(&db, body).unwrap();
    (root, db)
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_user_version_mismatch_and_broken_foreign_keys_and_newer_db() {
    for (case, sql) in [
        ("user_version", "PRAGMA user_version = 3"),
        (
            "foreign_key_check",
            "INSERT INTO messages(session_id, role, body_md, ts) VALUES (999, 'user', 'orphan', 1)",
        ),
        (
            "newer",
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (99, 'future', 1); PRAGMA user_version = 99",
        ),
    ] {
        let r = rig().await;
        Connection::open(&r.db).unwrap().execute_batch(sql).unwrap();
        let id = age::x25519::Identity::generate();
        let archive = forge_archive(&r, &id);
        let key = identity_file(&r.top, "id.txt", &id);
        let (root, db) = target_with_live(&r.top, "LIVE-DB");
        let o = restore_cmd(&archive, &key, &db, &root).output().unwrap();
        assert!(!o.status.success(), "{case}: restore must refuse");
        assert!(stderr(&o).contains(case), "{case}: {}", stderr(&o));
        assert_eq!(std::fs::read(&db).unwrap(), "LIVE-DB".as_bytes(), "{case}");
        let names: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "{case}: nothing moved or created");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_while_agentd_socket_is_alive() {
    let r = rig().await;
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let key = identity_file(&r.top, "id.txt", &id);
    let (root, db) = target_with_live(&r.top, "LIVE-DB");
    let sock = r.top.join("agentd.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let o = restore_cmd(&archive, &key, &db, &root)
        .arg("--socket")
        .arg(&sock)
        .output()
        .unwrap();
    assert!(!o.status.success(), "agentd is alive: restore is forbidden");
    assert!(stderr(&o).contains("agentd"), "{}", stderr(&o));
    assert_eq!(std::fs::read(&db).unwrap(), "LIVE-DB".as_bytes());
    drop(listener);
    // The socket remained as a file but nobody listens: agentd is stopped, restore proceeds.
    let o = restore_cmd(&archive, &key, &db, &root)
        .arg("--socket")
        .arg(&sock)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_from_pre_migration_snapshot_moves_live_files_to_bak() {
    let r = rig().await;
    let snap_dir = r.top.join("migration-backups");
    let snap = {
        let c = Connection::open(&r.db).unwrap();
        agentd_db::snapshot::take(&c, &snap_dir, 10, "20261107").unwrap()
    };
    // After the snapshot the data changed: restore must return the snapshot state.
    Connection::open(&r.db)
        .unwrap()
        .execute("UPDATE sessions SET name = 'Changed after snapshot'", [])
        .unwrap();
    std::fs::write(r.root.join("sessions.db-wal"), "OLD-WAL").unwrap();
    std::fs::write(r.root.join("sessions.db-shm"), "OLD-SHM").unwrap();
    let o = agentd()
        .args(["restore", "--snapshot"])
        .arg(&snap)
        .arg("--db")
        .arg(&r.db)
        .arg("--root")
        .arg(&r.root)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let c = Connection::open(&r.db).unwrap();
    let name: String = c
        .query_row("SELECT name FROM sessions", [], |x| x.get(0))
        .unwrap();
    assert_eq!(name, "Session");
    let names: Vec<String> = std::fs::read_dir(&r.root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for p in [
        "sessions.db.bak-",
        "sessions.db-wal.bak-",
        "sessions.db-shm.bak-",
    ] {
        assert!(names.iter().any(|n| n.starts_with(p)), "{p}: {names:?}");
    }
    assert!(!names.contains(&"sessions.db-wal".to_string()));
    assert_eq!(
        std::fs::metadata(&r.db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(snap.exists(), "snapshot stays in place");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_from_bad_snapshot_changes_nothing() {
    let r = rig().await;
    let bad = r.top.join("pre-migration-9.db");
    std::fs::write(&bad, "this is not sqlite").unwrap();
    let before = std::fs::read(&r.db).unwrap();
    let o = agentd()
        .args(["restore", "--snapshot"])
        .arg(&bad)
        .arg("--db")
        .arg(&r.db)
        .arg("--root")
        .arg(&r.root)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert_eq!(std::fs::read(&r.db).unwrap(), before);
    assert!(
        !std::fs::read_dir(&r.root)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".bak-")),
        "nothing moved"
    );
}

// ---------- S10-10: the whole scenario ----------

/// All tables in full: name -> sorted rows as `quote()` text.
fn dump(db: &Path) -> Vec<(String, Vec<String>)> {
    let c = Connection::open(db).unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |x| x.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables
        .into_iter()
        .map(|t| {
            let cols: Vec<String> = c
                .prepare(&format!("PRAGMA table_info({t})"))
                .unwrap()
                .query_map([], |x| x.get(1))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let expr = cols
                .iter()
                .map(|n| format!("quote(\"{n}\")"))
                .collect::<Vec<_>>()
                .join("||'|'||");
            let mut rows: Vec<String> = c
                .prepare(&format!("SELECT {expr} FROM \"{t}\""))
                .unwrap()
                .query_map([], |x| x.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (t, rows)
        })
        .collect()
}

fn pragma_i64(db: &Path, p: &str) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row(&format!("PRAGMA {p}"), [], |x| x.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn scenario_backup_loss_restore_start_keeps_every_row() {
    let r = rig().await;
    let before = dump(&r.db);
    assert!(
        before.iter().map(|(_, v)| v.len()).sum::<usize>() > 10,
        "the fixture has something to compare"
    );
    let uv = pragma_i64(&r.db, "user_version");
    let id = age::x25519::Identity::generate();
    let archive = make_backup(&r, &id);
    let key = identity_file(&r.top, "id.txt", &id);
    // DB loss: move to *.bak (not rm), together with -wal/-shm.
    for n in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let p = r.root.join(n);
        if p.exists() {
            std::fs::rename(&p, r.root.join(format!("{n}.lost"))).unwrap();
        }
    }
    std::fs::rename(r.root.join("AGENT.md"), r.root.join("AGENT.md.lost")).unwrap();
    assert!(!r.db.exists());
    let o = restore_cmd(&archive, &key, &r.db, &r.root)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(dump(&r.db), before, "data before and after match");
    assert_eq!(pragma_i64(&r.db, "user_version"), uv);
    assert_eq!(
        pragma_i64(&r.db, "user_version"),
        Connection::open(&r.db)
            .unwrap()
            .query_row("SELECT max(version) FROM schema_migrations", [], |x| x
                .get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        Connection::open(&r.db)
            .unwrap()
            .query_row("PRAGMA integrity_check", [], |x| x.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        std::fs::read(r.root.join("AGENT.md")).unwrap(),
        "Prompt".as_bytes()
    );
    // The core comes up on the restored DB: the migration runner changes nothing, readers see the data.
    let d = Arc::new(Db::open(DbConfig::new(&r.db)).unwrap());
    let n: i64 = d
        .read(|c| c.query_row("SELECT count(*) FROM messages", [], |x| x.get(0)))
        .await
        .unwrap();
    assert_eq!(n, 2);
}

/// Assumption "<= 60 s for a 100 MB DB": size is set by `S10_RESTORE_MB` (default 8 MB so a debug run is fast);
/// the 100 MB measurement is taken in release and put in the log.
#[tokio::test(flavor = "multi_thread")]
async fn restore_time_is_within_sixty_seconds() {
    let mb: i64 = std::env::var("S10_RESTORE_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let r = rig().await;
    {
        let c = Connection::open(&r.db).unwrap();
        c.execute_batch("BEGIN").unwrap();
        // 1 row ~ 1 KB of random-looking text (does not compress to nothing).
        for i in 0..(mb * 1024) {
            c.execute(
                "INSERT INTO messages(session_id, role, body_md, ts) VALUES (1, 'user', hex(randomblob(512)), ?1)",
                [i],
            )
            .unwrap();
        }
        c.execute_batch("COMMIT").unwrap();
    }
    let size = std::fs::metadata(&r.db).unwrap().len();
    let id = age::x25519::Identity::generate();
    let key = identity_file(&r.top, "id.txt", &id);
    let t = Instant::now();
    let archive = make_backup(&r, &id);
    let backup_s = t.elapsed().as_secs_f64();
    let (root, db) = target_with_live(&r.top, "LIVE-DB");
    let t = Instant::now();
    let o = restore_cmd(&archive, &key, &db, &root).output().unwrap();
    let restore_s = t.elapsed().as_secs_f64();
    assert!(o.status.success(), "{}", stderr(&o));
    eprintln!(
        "MEASUREMENT: DB {} MB, backup {backup_s:.2} s, restore {restore_s:.2} s",
        size / 1_000_000
    );
    assert!(restore_s < 60.0, "restore {restore_s:.1} s");
    assert!(pragma_i64(&db, "user_version") > 0);
}
