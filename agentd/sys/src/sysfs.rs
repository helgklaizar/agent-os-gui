//! System indicators from sysfs and the backup directory: network, battery, VPN, age of the last backup. Read-only.
//! The sysfs root is a parameter (`/` in production, a fixture tree in tests). No sensor means `None`, never an invented value.

use std::path::Path;
use std::time::UNIX_EPOCH;

/// Network interface: name and `operstate` (`up`, `down`, `unknown`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub operstate: String,
}

/// Interfaces from `<root>/sys/class/net` except `lo`, by name. No directory means `None`.
#[must_use]
pub fn ifaces(root: &Path) -> Option<Vec<Iface>> {
    let dir = std::fs::read_dir(root.join("sys/class/net")).ok()?;
    let mut out: Vec<Iface> = dir
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name == "lo" {
                return None;
            }
            let operstate = std::fs::read_to_string(e.path().join("operstate"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            Some(Iface { name, operstate })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Some(out)
}

/// Names of VPN and tunnel interfaces.
#[must_use]
pub fn is_vpn(name: &str) -> bool {
    [
        "wg",
        "tun",
        "tap",
        "tailscale",
        "ppp",
        "zt",
        "nordlynx",
        "proton",
        "mullvad",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// Virtual container and bridge interfaces: not counted as a link.
#[must_use]
pub fn is_virtual(name: &str) -> bool {
    ["docker", "veth", "br-", "virbr", "lxc", "cni", "flannel"]
        .iter()
        .any(|p| name.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// First `BAT*` battery in `<root>/sys/class/power_supply`; no battery (desktop PC) means `None`.
#[must_use]
pub fn battery(root: &Path) -> Option<Battery> {
    let dir = std::fs::read_dir(root.join("sys/class/power_supply")).ok()?;
    let mut names: Vec<_> = dir
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        .map(|e| e.path())
        .collect();
    names.sort();
    let p = names.first()?;
    let percent = std::fs::read_to_string(p.join("capacity"))
        .ok()?
        .trim()
        .parse::<u8>()
        .ok()?
        .min(100);
    let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
    let charging = matches!(status.trim(), "Charging" | "Full");
    Some(Battery { percent, charging })
}

/// Modification time (unix seconds) of the newest `backup-*.tar.age` in the directory; no archives means `None`.
#[must_use]
pub fn newest_backup(dir: &Path) -> Option<i64> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("backup-") && n.ends_with(".tar.age")
        })
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(base: &Path, rel: &str, text: &str) {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn ifaces_skip_loopback_and_missing_dir_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert!(ifaces(d.path()).is_none());
        put(d.path(), "sys/class/net/lo/operstate", "unknown\n");
        put(d.path(), "sys/class/net/eth0/operstate", "up\n");
        put(d.path(), "sys/class/net/wg0/operstate", "unknown\n");
        let v = ifaces(d.path()).unwrap();
        let names: Vec<&str> = v.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["eth0", "wg0"]);
        assert_eq!(v[0].operstate, "up");
    }

    #[test]
    fn vpn_and_virtual_names() {
        for n in ["wg0", "tun0", "tailscale0", "nordlynx"] {
            assert!(is_vpn(n), "{n}");
        }
        for n in ["eth0", "wlan0", "enp3s0", "docker0"] {
            assert!(!is_vpn(n), "{n}");
        }
        for n in ["docker0", "veth12ab", "br-1f2e", "virbr0"] {
            assert!(is_virtual(n), "{n}");
        }
        assert!(!is_virtual("eth0"));
    }

    #[test]
    fn battery_reads_first_bat_and_charging() {
        let d = tempfile::tempdir().unwrap();
        assert!(battery(d.path()).is_none());
        put(d.path(), "sys/class/power_supply/AC/online", "1\n");
        assert!(battery(d.path()).is_none(), "adapter without a battery");
        put(d.path(), "sys/class/power_supply/BAT0/capacity", "42\n");
        put(d.path(), "sys/class/power_supply/BAT0/status", "Charging\n");
        assert_eq!(
            battery(d.path()),
            Some(Battery {
                percent: 42,
                charging: true
            })
        );
        put(
            d.path(),
            "sys/class/power_supply/BAT0/capacity",
            "garbage\n",
        );
        assert!(
            battery(d.path()).is_none(),
            "unreadable value means no data"
        );
    }

    #[test]
    fn newest_backup_ignores_foreign_files() {
        let d = tempfile::tempdir().unwrap();
        assert!(newest_backup(d.path()).is_none());
        put(d.path(), "notes.txt", "x");
        assert!(newest_backup(d.path()).is_none());
        put(d.path(), "backup-1.tar.age", "x");
        assert!(newest_backup(d.path()).is_some());
        assert!(newest_backup(&d.path().join("missing")).is_none());
    }
}
