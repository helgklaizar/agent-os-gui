//! Disk encryption check: is the DB partition encrypted (LUKS/dm-crypt) or the directory (fscrypt).
//!
//! Missing encryption is a warning, not a block. Data comes from `/proc/self/mountinfo`, `/sys` and
//! `lsattr -d` (flag `E`); the sources are gathered in [`DiskProbe`], so parsing is tested on prepared data.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SLAVE_DEPTH: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// dm-crypt with a LUKS header (`CRYPT-LUKS...` in `dm/uuid`).
    Luks,
    /// dm-crypt without LUKS (`CRYPT-PLAIN...` and others).
    DmCrypt,
    /// fscrypt: the directory has the `E` flag in `lsattr`.
    Fscrypt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskEncryption {
    Encrypted(Mechanism),
    NotEncrypted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskWarning {
    NotEncrypted,
    Undetermined,
}

impl fmt::Display for DiskWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskWarning::NotEncrypted => {
                f.write_str("DB partition is not encrypted (LUKS or fscrypt required)")
            }
            DiskWarning::Undetermined => {
                f.write_str("could not determine whether the DB partition is encrypted")
            }
        }
    }
}

/// Data sources for determining encryption.
#[derive(Debug, Clone)]
pub struct DiskProbe {
    /// Contents of `/proc/self/mountinfo`.
    pub mountinfo: String,
    /// Root of sysfs (`/sys`).
    pub sysfs_root: PathBuf,
    /// Root under which `dev/...` lives (`/`); needed to resolve `/dev/mapper/...` on filesystems without a device number (btrfs).
    pub dev_root: PathBuf,
    /// Output of `lsattr -d <directory>`; `None` - the command is unavailable or unsupported.
    pub lsattr_output: Option<String>,
}

impl DiskProbe {
    /// Live system sources. Outside Linux part of the data is empty and the result is `Unknown`.
    pub fn system(path: &Path) -> DiskProbe {
        // fscrypt and lsattr exist only on Linux; on other systems the command is not run
        let lsattr_output = cfg!(target_os = "linux")
            .then(|| {
                std::process::Command::new("lsattr")
                    .arg("-d")
                    .arg(path)
                    .output()
                    .ok()
            })
            .flatten()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        DiskProbe {
            mountinfo: fs::read_to_string("/proc/self/mountinfo").unwrap_or_default(),
            sysfs_root: PathBuf::from("/sys"),
            dev_root: PathBuf::from("/"),
            lsattr_output,
        }
    }
}

struct Mount {
    major_minor: String,
    mount_point: PathBuf,
    source: String,
}

fn unescape_octal(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let oct = b
            .get(i + 1..i + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u8::from_str_radix(d, 8).ok());
        match (b[i], oct) {
            (b'\\', Some(v)) => {
                out.push(v);
                i += 4;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let l: Vec<&str> = left.split_whitespace().collect();
            let r: Vec<&str> = right.split_whitespace().collect();
            if l.len() < 6 || r.len() < 2 {
                return None;
            }
            Some(Mount {
                major_minor: l[2].to_string(),
                mount_point: PathBuf::from(unescape_octal(l[4])),
                source: unescape_octal(r[1]),
            })
        })
        .collect()
}

/// Block device directory in sysfs: by `MAJ:MIN`, otherwise by the mount source `/dev/...`.
fn block_dir(probe: &DiskProbe, mount: &Mount) -> Option<PathBuf> {
    let by_number = probe.sysfs_root.join("dev/block").join(&mount.major_minor);
    if by_number.exists() {
        return Some(by_number);
    }
    let dev = mount
        .source
        .strip_prefix('/')
        .filter(|s| s.starts_with("dev/"))?;
    let real = fs::canonicalize(probe.dev_root.join(dev)).ok()?;
    let by_name = probe.sysfs_root.join("class/block").join(real.file_name()?);
    by_name.exists().then_some(by_name)
}

fn crypt_mechanism(probe: &DiskProbe, dir: &Path, depth: u8) -> Option<Mechanism> {
    if let Ok(uuid) = fs::read_to_string(dir.join("dm/uuid")) {
        let uuid = uuid.trim();
        if uuid.starts_with("CRYPT-LUKS") {
            return Some(Mechanism::Luks);
        }
        if uuid.starts_with("CRYPT-") {
            return Some(Mechanism::DmCrypt);
        }
    }
    if depth >= MAX_SLAVE_DEPTH {
        return None;
    }
    let slaves = fs::read_dir(dir.join("slaves")).ok()?;
    slaves
        .flatten()
        .filter_map(|e| {
            let below = probe.sysfs_root.join("class/block").join(e.file_name());
            below
                .exists()
                .then(|| crypt_mechanism(probe, &below, depth + 1))
                .flatten()
        })
        .next()
}

fn lsattr_encrypted(output: &str) -> bool {
    output
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .is_some_and(|flags| flags.contains('E'))
}

/// Determines the encryption of the partition holding `path`.
pub fn detect_disk_encryption(probe: &DiskProbe, path: &Path) -> DiskEncryption {
    let mounts = parse_mountinfo(&probe.mountinfo);
    let mut best: Option<&Mount> = None;
    for m in mounts.iter().filter(|m| path.starts_with(&m.mount_point)) {
        // a deeper mount point wins; on a tie (overlay mount) the later one wins
        if best.is_none_or(|b| {
            m.mount_point.components().count() >= b.mount_point.components().count()
        }) {
            best = Some(m);
        }
    }
    let Some(mount) = best else {
        return DiskEncryption::Unknown;
    };

    let block = block_dir(probe, mount);
    if let Some(mech) = block.as_deref().and_then(|d| crypt_mechanism(probe, d, 0)) {
        return DiskEncryption::Encrypted(mech);
    }
    if probe.lsattr_output.as_deref().is_some_and(lsattr_encrypted) {
        return DiskEncryption::Encrypted(Mechanism::Fscrypt);
    }
    if block.is_some() {
        DiskEncryption::NotEncrypted
    } else {
        DiskEncryption::Unknown
    }
}

/// Encryption of the partition with `path` from system data; a nonexistent path is replaced by the nearest existing ancestor.
pub fn disk_encryption_status(path: &Path) -> DiskEncryption {
    let mut probe_path = path;
    let real = loop {
        if let Ok(p) = fs::canonicalize(probe_path) {
            break p;
        }
        match probe_path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => probe_path = parent,
            _ => return DiskEncryption::Unknown,
        }
    };
    detect_disk_encryption(&DiskProbe::system(&real), &real)
}

/// User warning; `None` - encrypted.
pub fn disk_encryption_warning(status: DiskEncryption) -> Option<DiskWarning> {
    match status {
        DiskEncryption::Encrypted(_) => None,
        DiskEncryption::NotEncrypted => Some(DiskWarning::NotEncrypted),
        DiskEncryption::Unknown => Some(DiskWarning::Undetermined),
    }
}
