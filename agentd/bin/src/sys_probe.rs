//! Data source for the `net`, `battery`, `vpn`, `backup` indicators (A7-07): sysfs and the `~/.agentos/export` directory.
//! The sysfs root is `/`; in a test build it is replaced by `AGENTD_SYSFS_ROOT` (a fixture tree).

use std::path::PathBuf;

use agentd_core::{Link, SystemProbe};
use agentd_sys::sysfs;

pub(crate) struct FsProbe {
    root: PathBuf,
    export: PathBuf,
}

impl FsProbe {
    pub(crate) fn new(agentos: &std::path::Path) -> FsProbe {
        // Root override is for the test build only (in a release build `manual_clock` returns `None`).
        let root = agentd_core::manual_clock(0)
            .and(std::env::var_os("AGENTD_SYSFS_ROOT"))
            .map_or_else(|| PathBuf::from("/"), PathBuf::from);
        FsProbe {
            root,
            export: agentos.join("export"),
        }
    }
}

impl SystemProbe for FsProbe {
    fn links(&self) -> Option<Vec<Link>> {
        Some(
            sysfs::ifaces(&self.root)?
                .into_iter()
                .map(|i| Link {
                    vpn: sysfs::is_vpn(&i.name),
                    virtual_: sysfs::is_virtual(&i.name),
                    state: i.operstate,
                    name: i.name,
                })
                .collect(),
        )
    }

    fn battery(&self) -> Option<(u8, bool)> {
        sysfs::battery(&self.root).map(|b| (b.percent, b.charging))
    }

    fn last_backup(&self) -> Option<i64> {
        sysfs::newest_backup(&self.export)
    }
}
