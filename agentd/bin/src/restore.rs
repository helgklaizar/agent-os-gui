//! `agentd restore` (A7-09, S10-DB-03): restore from a `backup` archive (`age` secret key) or from a snapshot
//! `pre-migration-VERSION.db`. First it checks that `agentd` is stopped (the socket does not answer), then everything is prepared in
//! temporary files next to the target and verified (`integrity_check`, `foreign_key_check`, `user_version` against
//! `schema_migrations`, version not newer than the code); only then the live files (`sessions.db`, `-wal`, `-shm`, `index.db`,
//! `AGENT.md`, matching `memory/` files) are moved to `*.bak-YYYYMMDD` and the new ones take their place. Nothing is deleted;
//! on any error before the move the live files stay in place.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::archive::{DB_NAME, check_db_bytes, open_archive};

#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Archive {
        archive: PathBuf,
        identity: PathBuf,
    },
    /// Pre-migration snapshot: a plain SQLite file, no key needed.
    Snapshot(PathBuf),
}

fn write_new(path: &Path, data: &[u8]) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".restore-tmp");
    let tmp = PathBuf::from(tmp);
    drop(fs::remove_file(&tmp));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(tmp)
}

/// `agentd` is alive if the socket accepts a connection; a socket file with no listener means the core is stopped.
fn ensure_stopped(socket: Option<&Path>) -> Result<()> {
    if socket.is_some_and(|s| UnixStream::connect(s).is_ok()) {
        bail!("agentd is running (socket answers): stop it before restoring");
    }
    Ok(())
}

fn load(source: &Source) -> Result<Vec<(String, Vec<u8>)>> {
    match source {
        Source::Archive { archive, identity } => open_archive(archive, identity),
        Source::Snapshot(path) => {
            let m = fs::symlink_metadata(path).context("snapshot not found")?;
            if !m.is_file() {
                bail!("snapshot must be a regular file");
            }
            Ok(vec![(
                DB_NAME.to_string(),
                fs::read(path).context("snapshot not read")?,
            )])
        }
    }
}

/// Returns the number of live files moved to `*.bak-YYYYMMDD`.
pub fn run(source: &Source, db: &Path, root: &Path, socket: Option<&Path>) -> Result<usize> {
    ensure_stopped(socket)?;
    let files = load(source)?;
    let target_of = |name: &str| -> PathBuf {
        if name == DB_NAME {
            db.to_path_buf()
        } else {
            root.join(name)
        }
    };
    // Stage 0: the DB is verified in a temporary directory (removed by the library), nothing is written to the target disk yet.
    let db_dir = db.parent().unwrap_or_else(|| Path::new("."));
    for (name, data) in &files {
        if name == DB_NAME {
            check_db_bytes(data, db_dir).context("restored DB failed verification")?;
        }
    }
    // Stage 1: new files next to the targets.
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let staged_all = (|| -> Result<()> {
        for (name, data) in &files {
            let target = target_of(name);
            let tmp = write_new(&target, data)?;
            staged.push((tmp, target));
        }
        Ok(())
    })();
    if let Err(e) = staged_all {
        for (tmp, _) in &staged {
            // The temporary files are our own and not yet used; live files are untouched.
            drop(fs::rename(
                tmp,
                agentd_db::snapshot::backup_name(tmp, "restore-failed"),
            ));
        }
        return Err(e);
    }
    // Stage 2: live files go to *.bak, new ones take their place.
    let stamp = agentd_db::snapshot::today_stamp();
    let mut live: Vec<PathBuf> = vec![];
    for (_, target) in &staged {
        live.push(target.clone());
        if target == db {
            for suffix in ["-wal", "-shm"] {
                let mut s = target.as_os_str().to_owned();
                s.push(suffix);
                live.push(PathBuf::from(s));
            }
        }
    }
    let mut moved = 0;
    for path in live {
        if fs::symlink_metadata(&path).is_ok() {
            fs::rename(&path, agentd_db::snapshot::backup_name(&path, &stamp))
                .with_context(|| format!("moving {} to *.bak", path.display()))?;
            moved += 1;
        }
    }
    for (tmp, target) in staged {
        fs::rename(&tmp, &target).with_context(|| format!("installing {}", target.display()))?;
    }
    Ok(moved)
}
