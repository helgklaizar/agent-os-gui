//! System and infrastructure indicators: `net`, `battery`, `vpn`, `backup`. Data sources sit behind the
//! `SystemProbe` port (the binary reads sysfs and the backup directory); this module only evaluates state. No sensor — `nodata`.
//! Thresholds: no link — red; battery <= 30 % yellow, <= 15 % red (charging is fine);
//! backup older than 26 h yellow, older than 50 h red.

use std::sync::Arc;

use serde_json::json;

use super::{IndicatorSource, IndicatorState, Reading};
use crate::{Clock, PortFuture, SettingsStore};

/// A network interface in a form suitable for evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub name: String,
    pub vpn: bool,
    /// Container or bridge interface: does not count as a link.
    pub virtual_: bool,
    /// `operstate`: `up`, `down`, `unknown`, …
    pub state: String,
}

pub trait SystemProbe: Send + Sync + 'static {
    /// Interfaces without `lo`; `None` — no information.
    fn links(&self) -> Option<Vec<Link>>;
    /// `(percent, charging)`; `None` — no battery.
    fn battery(&self) -> Option<(u8, bool)>;
    /// Time of the newest backup archive (unix seconds); `None` — no archives.
    fn last_backup(&self) -> Option<i64>;
}

pub const BACKUP_WARN_SECS: i64 = 26 * 3600;
pub const BACKUP_BAD_SECS: i64 = 50 * 3600;
/// Restore check (`backup.last_restore_test_at`) older than 35 days — red.
pub const RESTORE_TEST_BAD_SECS: i64 = 35 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Net,
    Battery,
    Vpn,
    Backup,
}

pub struct SystemIndicator {
    id: &'static str,
    kind: Kind,
    probe: Arc<dyn SystemProbe>,
    clock: Arc<dyn Clock>,
    settings: Arc<dyn SettingsStore>,
}

impl SystemIndicator {
    /// `net`, `battery`, `vpn`, `backup` on one source.
    #[must_use]
    pub fn all(
        probe: &Arc<dyn SystemProbe>,
        clock: &Arc<dyn Clock>,
        settings: &Arc<dyn SettingsStore>,
    ) -> Vec<SystemIndicator> {
        [
            ("net", Kind::Net),
            ("battery", Kind::Battery),
            ("vpn", Kind::Vpn),
            ("backup", Kind::Backup),
        ]
        .into_iter()
        .map(|(id, kind)| SystemIndicator {
            id,
            kind,
            probe: probe.clone(),
            clock: clock.clone(),
            settings: settings.clone(),
        })
        .collect()
    }
}

fn nodata(text: &str) -> Reading {
    super::engine::nodata(text)
}

fn with(state: IndicatorState, value: serde_json::Value, text: String) -> Reading {
    Reading {
        state,
        value: Some(value),
        text: Some(text),
    }
}

/// Network: at least one physical interface `up` — ok; interfaces exist but no link — red.
#[must_use]
pub fn grade_net(links: Option<&[Link]>) -> Reading {
    let Some(links) = links else {
        return nodata("no network information");
    };
    let phys: Vec<&Link> = links.iter().filter(|l| !l.vpn && !l.virtual_).collect();
    if phys.is_empty() {
        return nodata("no network interfaces");
    }
    let up: Vec<&str> = phys
        .iter()
        .filter(|l| l.state == "up")
        .map(|l| l.name.as_str())
        .collect();
    if up.is_empty() {
        with(IndicatorState::Bad, json!({"up": 0}), "no link".into())
    } else {
        with(
            IndicatorState::Ok,
            json!({"up": up.len()}),
            format!("link: {}", up.join(", ")),
        )
    }
}

/// VPN: no interface — `nodata` (not configured); up (`up` or `unknown`, as WireGuard reports) — ok; down — yellow.
#[must_use]
pub fn grade_vpn(links: Option<&[Link]>) -> Reading {
    let vpn: Vec<&Link> = links.into_iter().flatten().filter(|l| l.vpn).collect();
    if vpn.is_empty() {
        return nodata("VPN not configured");
    }
    if vpn
        .iter()
        .any(|l| matches!(l.state.as_str(), "up" | "unknown"))
    {
        with(IndicatorState::Ok, json!({"up": true}), "VPN up".into())
    } else {
        with(
            IndicatorState::Warn,
            json!({"up": false}),
            "VPN down".into(),
        )
    }
}

#[must_use]
pub fn grade_battery(b: Option<(u8, bool)>) -> Reading {
    let Some((pct, charging)) = b else {
        return nodata("no battery");
    };
    let state = if charging || pct > 30 {
        IndicatorState::Ok
    } else if pct > 15 {
        IndicatorState::Warn
    } else {
        IndicatorState::Bad
    };
    with(state, json!(pct), format!("{pct}%"))
}

/// Backup age and (optionally) the last restore check time: older than 35 days — red.
#[must_use]
pub fn grade_backup(last: Option<i64>, now: i64, restore_test: Option<i64>) -> Reading {
    let Some(at) = last else {
        return nodata("no backups yet");
    };
    let age = (now - at).max(0);
    if let Some(t) = restore_test {
        let since = (now - t).max(0);
        if since > RESTORE_TEST_BAD_SECS {
            return with(
                IndicatorState::Bad,
                json!(age),
                format!("restore not verified for {} d", since / 86_400),
            );
        }
    }
    let state = if age > BACKUP_BAD_SECS {
        IndicatorState::Bad
    } else if age > BACKUP_WARN_SECS {
        IndicatorState::Warn
    } else {
        IndicatorState::Ok
    };
    with(
        state,
        json!(age),
        format!("last backup {} h ago", age / 3600),
    )
}

impl IndicatorSource for SystemIndicator {
    fn id(&self) -> &str {
        self.id
    }

    fn read(&self) -> PortFuture<'_, Reading> {
        Box::pin(async move {
            match self.kind {
                Kind::Net => grade_net(self.probe.links().as_deref()),
                Kind::Vpn => grade_vpn(self.probe.links().as_deref()),
                Kind::Battery => grade_battery(self.probe.battery()),
                Kind::Backup => {
                    let restore_test = self
                        .settings
                        .get("backup")
                        .await
                        .ok()
                        .and_then(|d| d.doc["last_restore_test_at"].as_i64());
                    grade_backup(
                        self.probe.last_backup(),
                        self.clock.now_unix(),
                        restore_test,
                    )
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, state: &str) -> Link {
        Link {
            name: name.into(),
            vpn: name.starts_with("wg"),
            virtual_: name.starts_with("docker"),
            state: state.into(),
        }
    }

    #[test]
    fn net_needs_a_physical_link() {
        assert_eq!(grade_net(None).state, IndicatorState::Nodata);
        assert_eq!(grade_net(Some(&[])).state, IndicatorState::Nodata);
        let only_virtual = [link("docker0", "up"), link("wg0", "up")];
        assert_eq!(grade_net(Some(&only_virtual)).state, IndicatorState::Nodata);
        assert_eq!(
            grade_net(Some(&[link("eth0", "down")])).state,
            IndicatorState::Bad
        );
        assert_eq!(
            grade_net(Some(&[link("eth0", "down"), link("wlan0", "up")])).state,
            IndicatorState::Ok
        );
    }

    #[test]
    fn vpn_states() {
        assert_eq!(grade_vpn(None).state, IndicatorState::Nodata);
        assert_eq!(
            grade_vpn(Some(&[link("eth0", "up")])).state,
            IndicatorState::Nodata
        );
        assert_eq!(
            grade_vpn(Some(&[link("wg0", "unknown")])).state,
            IndicatorState::Ok
        );
        assert_eq!(
            grade_vpn(Some(&[link("wg0", "down")])).state,
            IndicatorState::Warn
        );
    }

    #[test]
    fn battery_thresholds_and_charging() {
        assert_eq!(grade_battery(None).state, IndicatorState::Nodata);
        for (pct, charging, want) in [
            (31, false, IndicatorState::Ok),
            (30, false, IndicatorState::Warn),
            (16, false, IndicatorState::Warn),
            (15, false, IndicatorState::Bad),
            (1, false, IndicatorState::Bad),
            (5, true, IndicatorState::Ok),
        ] {
            assert_eq!(
                grade_battery(Some((pct, charging))).state,
                want,
                "{pct} {charging}"
            );
        }
    }

    #[test]
    fn restore_test_older_than_35_days_is_bad() {
        let now = 10_000_000;
        let fresh = Some(now - 3600);
        let day = 86_400;
        for (since, want) in [
            (34 * day, IndicatorState::Ok),
            (35 * day, IndicatorState::Ok),
            (35 * day + 1, IndicatorState::Bad),
            (36 * day, IndicatorState::Bad),
        ] {
            assert_eq!(
                grade_backup(fresh, now, Some(now - since)).state,
                want,
                "{since}"
            );
        }
        assert_eq!(
            grade_backup(None, now, Some(0)).state,
            IndicatorState::Nodata
        );
    }

    #[test]
    fn backup_age_thresholds() {
        let now = 1_000_000;
        assert_eq!(grade_backup(None, now, None).state, IndicatorState::Nodata);
        for (age, want) in [
            (0, IndicatorState::Ok),
            (BACKUP_WARN_SECS, IndicatorState::Ok),
            (BACKUP_WARN_SECS + 1, IndicatorState::Warn),
            (BACKUP_BAD_SECS, IndicatorState::Warn),
            (BACKUP_BAD_SECS + 1, IndicatorState::Bad),
        ] {
            assert_eq!(
                grade_backup(Some(now - age), now, None).state,
                want,
                "{age}"
            );
        }
        // A time from the future (clock skew) is not an error and not "very old".
        assert_eq!(
            grade_backup(Some(now + 500), now, None).state,
            IndicatorState::Ok
        );
    }
}
