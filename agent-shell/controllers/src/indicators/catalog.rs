//! Indicator catalogue with its thresholds. The threshold values come only from here.

use super::{ClickAction::Tooltip, Group, Manifest, Rule};

/// Default set. `cpu`, `ram`, `disk`, `temp` are driven by `bar.sys`, so their manifests are not here.
pub const DEFAULT_ENABLED: &[&str] = &[
    "cpu",
    "ram",
    "disk",
    "temp",
    "net.link",
    "ai.usage",
    "msg.unread",
    "infra.agentd",
];

const fn m(
    id: &'static str,
    group: Group,
    source: &'static str,
    interval_ms: Option<u64>,
    rule: Rule,
) -> Manifest {
    Manifest {
        id,
        group,
        source,
        interval_ms,
        rule,
        click_action: Tooltip,
    }
}

const fn above(warn: Option<f64>, bad: Option<f64>, inclusive: bool) -> Rule {
    Rule::Above {
        warn,
        bad,
        inclusive,
    }
}

const fn below(warn: Option<f64>, bad: Option<f64>, inclusive: bool) -> Rule {
    Rule::Below {
        warn,
        bad,
        inclusive,
    }
}

use super::State::{Bad, Warn};
use Group::{Ai, Infra, Messages, Module, System};

static CATALOG: &[Manifest] = &[
    // System. No link = red; value: 1 means a link is present, 0 means none.
    m(
        "net.link",
        System,
        "sysfs, NetworkManager",
        Some(2_000),
        below(None, Some(0.0), true),
    ),
    m("net.rate", System, "sysfs", Some(2_000), Rule::None),
    m(
        "battery",
        System,
        "UPower",
        Some(30_000),
        below(Some(30.0), Some(15.0), true),
    ),
    m(
        "audio.mic",
        System,
        "PipeWire",
        None,
        Rule::Flag { on: Warn },
    ),
    m("bluetooth", System, "D-Bus", None, Rule::None),
    m("layout", System, "D-Bus", None, Rule::None),
    m("sys.swap", System, "/proc", Some(5_000), Rule::None),
    // Value: load average / core count; > 1 yellow, > 2 red.
    m(
        "load",
        System,
        "/proc",
        Some(5_000),
        above(Some(1.0), Some(2.0), false),
    ),
    // Value: number of agent-* units in the failed state.
    m(
        "sys.units",
        System,
        "systemd --user",
        None,
        above(None, Some(1.0), true),
    ),
    m(
        "sys.updates",
        System,
        "apt",
        Some(3_600_000),
        Rule::Flag { on: Warn },
    ),
    m(
        "sys.reboot",
        System,
        "/var/run",
        Some(3_600_000),
        Rule::Flag { on: Warn },
    ),
    // AI. Value: remaining limit, %; computed only when the limit is known.
    m(
        "ai.usage",
        Ai,
        "agentd",
        Some(2_000),
        below(Some(40.0), Some(15.0), true),
    ),
    m("ai.cost", Ai, "agentd", Some(2_000), Rule::None),
    // Value: N − maxParallel; red when the queue exceeds maxParallel.
    m(
        "ai.queue",
        Ai,
        "agentd",
        None,
        above(None, Some(0.0), false),
    ),
    m("ai.tokens", Ai, "Ollama", Some(2_000), Rule::None),
    m(
        "ai.vram",
        Ai,
        "NVML",
        Some(2_000),
        above(Some(80.0), Some(95.0), true),
    ),
    m("ai.watch", Ai, "agent-watch", None, Rule::Flag { on: Bad }),
    // Messages. Counter subtypes come from the core as data.
    m("msg.unread", Messages, "agent-gw", None, Rule::None),
    // Value: minutes until the meeting; < 10 yellow.
    m(
        "msg.meeting",
        Messages,
        "calendar",
        Some(60_000),
        below(Some(10.0), None, false),
    ),
    // Infrastructure. Value: hours / days / number of failed jobs.
    m(
        "infra.backup",
        Infra,
        "agent-backup",
        Some(3_600_000),
        above(Some(26.0), Some(50.0), false),
    ),
    m(
        "infra.cert",
        Infra,
        "agent-sched",
        Some(86_400_000),
        below(Some(14.0), Some(5.0), true),
    ),
    m(
        "infra.cron",
        Infra,
        "agent-sched",
        Some(300_000),
        above(None, Some(1.0), true),
    ),
    m("infra.agentd", Infra, "connection", None, Rule::None),
    // Modules (outside 0.1): "QR" on a Whapi channel and a crashed container are red, a failed deploy is red.
    m(
        "msg.whapi",
        Module,
        "agent-gw",
        Some(300_000),
        Rule::Flag { on: Bad },
    ),
    m(
        "srv.disk",
        Module,
        "agent-sched (ssh)",
        Some(300_000),
        above(Some(80.0), Some(85.0), true),
    ),
    m(
        "srv.container",
        Module,
        "agent-sched (ssh)",
        Some(300_000),
        Rule::Flag { on: Bad },
    ),
    m(
        "git.deploy",
        Module,
        "agent-sched",
        None,
        Rule::Flag { on: Bad },
    ),
];

pub fn catalog() -> &'static [Manifest] {
    CATALOG
}

pub fn manifest(id: &str) -> Option<&'static Manifest> {
    CATALOG.iter().find(|m| m.id == id)
}
