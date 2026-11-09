//! Disk encryption check: LUKS and fscrypt, a warning without blocking.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use fake_agentd::disk::{detect_disk_encryption, disk_encryption_status, disk_encryption_warning};
use fake_agentd::{Db, DiskEncryption, DiskProbe, DiskWarning, Mechanism};
use tempfile::TempDir;

struct Fake {
    _dir: TempDir,
    root: PathBuf,
}

impl Fake {
    fn new() -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Fake { _dir: dir, root }
    }
    /// `/sys/dev/block/MAJ:MIN/dm/uuid`
    fn dm(&self, majmin: &str, uuid: &str) {
        let d = self.root.join("sys/dev/block").join(majmin).join("dm");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("uuid"), format!("{uuid}\n")).unwrap();
    }
    fn plain(&self, majmin: &str) {
        fs::create_dir_all(self.root.join("sys/dev/block").join(majmin)).unwrap();
    }
    /// `/sys/class/block/<name>` with a dm-uuid, referenced by `slaves/<name>` of another device
    fn named_dm(&self, name: &str, uuid: &str) {
        let d = self.root.join("sys/class/block").join(name).join("dm");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("uuid"), uuid).unwrap();
    }
    fn slave(&self, majmin: &str, slave: &str) {
        fs::create_dir_all(
            self.root
                .join("sys/dev/block")
                .join(majmin)
                .join("slaves")
                .join(slave),
        )
        .unwrap();
    }
    fn probe(&self, mountinfo: &str, lsattr: Option<&str>) -> DiskProbe {
        DiskProbe {
            mountinfo: mountinfo.to_string(),
            sysfs_root: self.root.join("sys"),
            dev_root: self.root.clone(),
            lsattr_output: lsattr.map(str::to_string),
        }
    }
}

fn status(p: &DiskProbe, path: &str) -> DiskEncryption {
    detect_disk_encryption(p, Path::new(path))
}

#[test]
fn disk_encryption_warning_luks_root_has_no_warning() {
    let f = Fake::new();
    f.dm("254:0", "CRYPT-LUKS2-0123-cryptroot");
    let p = f.probe(
        "36 1 254:0 / / rw,relatime - ext4 /dev/mapper/cryptroot rw\n",
        None,
    );
    let s = status(&p, "/home/u/.agentos/sessions.db");
    assert_eq!(s, DiskEncryption::Encrypted(Mechanism::Luks));
    assert_eq!(disk_encryption_warning(s), None);
}

#[test]
fn disk_encryption_warning_lvm_on_luks_is_detected() {
    let f = Fake::new();
    f.dm("254:1", "LVM-abcdef");
    f.slave("254:1", "dm-0");
    f.named_dm("dm-0", "CRYPT-LUKS1-9999-luks");
    let p = f.probe("36 1 254:1 / / rw - ext4 /dev/mapper/vg-root rw\n", None);
    assert_eq!(
        status(&p, "/home/u"),
        DiskEncryption::Encrypted(Mechanism::Luks)
    );
}

#[test]
fn disk_encryption_warning_plain_dm_crypt_counts_as_encrypted() {
    let f = Fake::new();
    f.dm("254:2", "CRYPT-PLAIN-xyz");
    let p = f.probe("36 1 254:2 / / rw - ext4 /dev/mapper/x rw\n", None);
    assert_eq!(
        status(&p, "/home/u"),
        DiskEncryption::Encrypted(Mechanism::DmCrypt)
    );
}

#[test]
fn disk_encryption_warning_plain_root_warns() {
    let f = Fake::new();
    f.plain("8:1");
    let p = f.probe("36 1 8:1 / / rw - ext4 /dev/sda1 rw\n", None);
    let s = status(&p, "/home/u/.agentos");
    assert_eq!(s, DiskEncryption::NotEncrypted);
    assert_eq!(disk_encryption_warning(s), Some(DiskWarning::NotEncrypted));
}

#[test]
fn disk_encryption_warning_lvm_without_crypt_slave_warns() {
    let f = Fake::new();
    f.dm("254:1", "LVM-abcdef");
    f.slave("254:1", "sda2");
    let p = f.probe("36 1 254:1 / / rw - ext4 /dev/mapper/vg-root rw\n", None);
    assert_eq!(status(&p, "/home/u"), DiskEncryption::NotEncrypted);
}

#[test]
fn disk_encryption_warning_longest_mount_wins() {
    let f = Fake::new();
    f.plain("8:1");
    f.dm("254:0", "CRYPT-LUKS2-home");
    let mi =
        "36 1 8:1 / / rw - ext4 /dev/sda1 rw\n40 36 254:0 / /home rw - ext4 /dev/mapper/home rw\n";
    let p = f.probe(mi, None);
    assert_eq!(
        status(&p, "/home/u/.agentos"),
        DiskEncryption::Encrypted(Mechanism::Luks)
    );
    assert_eq!(
        status(&p, "/homework/x"),
        DiskEncryption::NotEncrypted,
        "/homework is not under /home"
    );
    assert_eq!(status(&p, "/var/lib"), DiskEncryption::NotEncrypted);
}

#[test]
fn disk_encryption_warning_fscrypt_directory() {
    let f = Fake::new();
    f.plain("8:1");
    let mi = "36 1 8:1 / / rw - ext4 /dev/sda1 rw\n";
    let enc = f.probe(mi, Some("--------------E------- /home/u/.agentos\n"));
    let s = status(&enc, "/home/u/.agentos");
    assert_eq!(s, DiskEncryption::Encrypted(Mechanism::Fscrypt));
    assert_eq!(disk_encryption_warning(s), None);
    let none = f.probe(mi, Some("--------------e------- /home/u/.agentos\n"));
    assert_eq!(
        status(&none, "/home/u/.agentos"),
        DiskEncryption::NotEncrypted,
        "flag e (extents) is not encryption"
    );
}

#[test]
fn disk_encryption_warning_unknown_is_warning_not_error() {
    let f = Fake::new();
    let p = f.probe("", None);
    let s = status(&p, "/home/u");
    assert_eq!(s, DiskEncryption::Unknown);
    assert_eq!(disk_encryption_warning(s), Some(DiskWarning::Undetermined));
    let garbage = f.probe("not a mountinfo line\n", None);
    assert_eq!(status(&garbage, "/home/u"), DiskEncryption::Unknown);
}

#[test]
fn disk_encryption_warning_btrfs_on_luks_via_dev_source() {
    let f = Fake::new();
    f.named_dm("dm-0", "CRYPT-LUKS2-btrfs");
    fs::create_dir_all(f.root.join("dev/mapper")).unwrap();
    symlink("../dm-0", f.root.join("dev/mapper/cryptroot")).unwrap();
    fs::write(f.root.join("dev/dm-0"), "").unwrap();
    let p = f.probe("36 1 0:30 /@ / rw - btrfs /dev/mapper/cryptroot rw\n", None);
    assert_eq!(
        status(&p, "/home/u"),
        DiskEncryption::Encrypted(Mechanism::Luks)
    );
}

#[test]
fn disk_encryption_warning_escaped_mount_point() {
    let f = Fake::new();
    f.plain("8:1");
    f.dm("254:5", "CRYPT-LUKS2-d");
    let mi = "36 1 8:1 / / rw - ext4 /dev/sda1 rw\n41 36 254:5 / /mnt/my\\040disk rw - ext4 /dev/mapper/d rw\n";
    let p = f.probe(mi, None);
    assert_eq!(
        status(&p, "/mnt/my disk/db"),
        DiskEncryption::Encrypted(Mechanism::Luks)
    );
}

#[test]
fn disk_encryption_warning_does_not_block_open() {
    let (d, path) = common::temp_db_path();
    let db = Db::open(&path).expect("the warning does not block opening the DB");
    let expected = disk_encryption_status(d.path());
    let rep = db.storage_report();
    assert_eq!(
        rep.encryption, expected,
        "Db::open uses the disk check of the DB directory"
    );
    assert_eq!(rep.warning, disk_encryption_warning(expected));
    assert!(rep.warning.is_some() || matches!(expected, DiskEncryption::Encrypted(_)));
}
