//! Module types for `ModulesController`: a `modules` table row, widget and button data, sandbox violations.

use serde_json::Value;

use crate::manifest::Placement;

/// Module name length in the view (characters).
pub const MAX_TITLE: usize = 128;
/// Widget or button text length (characters).
pub const MAX_BADGE_TEXT: usize = 64;

/// Strip control, invisible and bidi characters and limit the length: text from a foreign manifest or from a module.
pub fn plain_text(raw: &str, max: usize, ellipsis: bool) -> String {
    let mut out: String = raw
        .chars()
        .filter(|c| {
            !c.is_control()
                && !('\u{200B}'..='\u{200F}').contains(c)
                && !('\u{202A}'..='\u{202E}').contains(c)
                && !('\u{2066}'..='\u{2069}').contains(c)
        })
        .take(max + 1)
        .collect();
    if out.chars().count() > max {
        out = out.chars().take(max).collect();
        if ellipsis {
            out.push('…');
        }
    }
    out
}

/// A `modules` table row as the core returns it (`GET /modules`, topic `modules`, docs/reference/api.md §5.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRecord {
    pub id: String,
    pub name: String,
    pub manifest_json: String,
    pub installed: bool,
    pub enabled: bool,
    pub placement: Placement,
    pub config_json: String,
}

impl ModuleRecord {
    /// From core JSON. A frame without `id`, with a foreign placement, or not an object is skipped (docs/reference/api.md §2): `None`.
    /// `installed` and `enabled` default to false; the manifest stays text and is validated on display.
    pub fn from_value(v: &Value) -> Option<ModuleRecord> {
        let o = v.as_object()?;
        let id = o.get("id")?.as_str()?.to_string();
        let placement = match o.get("placement")?.as_str()? {
            "panel" => Placement::Panel,
            "widget" => Placement::Widget,
            "button" => Placement::Button,
            _ => return None,
        };
        let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(ModuleRecord {
            name: o
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            manifest_json: o.get("manifest").map(Value::to_string).unwrap_or_default(),
            installed: flag("installed"),
            enabled: flag("enabled"),
            placement,
            config_json: o.get("config").map(Value::to_string).unwrap_or_default(),
            id,
        })
    }
}

/// Kind of sandbox violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationKind {
    ExternalOrigin,
    AgentdSocket,
    FileScheme,
    BridgeMethod,
    Navigation,
}

impl ViolationKind {
    /// Stable kind name for the log.
    pub fn as_str(self) -> &'static str {
        match self {
            ViolationKind::ExternalOrigin => "external_origin",
            ViolationKind::AgentdSocket => "agentd_socket",
            ViolationKind::FileScheme => "file_scheme",
            ViolationKind::BridgeMethod => "bridge_method",
            ViolationKind::Navigation => "navigation",
        }
    }
}

/// A module's attempt to escape the sandbox. `detail` is for the local log only and never goes to the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViolationReport {
    pub module_id: String,
    pub kind: ViolationKind,
    pub detail: String,
}

/// Closed set of widget and button icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Info,
    Ok,
    Warning,
    Error,
    Clock,
}

/// Closed widget colour palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Ok,
    Warn,
    Bad,
    Muted,
}

/// What a module may show in the bar or on a button: text, number, an icon from the set, a colour from the palette. Not HTML.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Badge {
    pub text: String,
    pub number: Option<f64>,
    pub icon: Option<Icon>,
    pub tone: Option<Tone>,
}

impl Badge {
    /// From JSON sent by a module: extra fields are dropped, text is sanitised, unknown icons and colours are discarded.
    pub fn from_module_json(json: &str) -> Badge {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(json) else {
            return Badge::default();
        };
        let word = |k: &str| obj.get(k).and_then(Value::as_str);
        Badge {
            text: word("text")
                .map(|t| plain_text(t, MAX_BADGE_TEXT, false))
                .unwrap_or_default(),
            number: obj
                .get("number")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite()),
            icon: word("icon").and_then(|n| match n {
                "info" => Some(Icon::Info),
                "ok" => Some(Icon::Ok),
                "warning" => Some(Icon::Warning),
                "error" => Some(Icon::Error),
                "clock" => Some(Icon::Clock),
                _ => None,
            }),
            tone: word("tone").and_then(|n| match n {
                "accent" => Some(Tone::Accent),
                "ok" => Some(Tone::Ok),
                "warn" => Some(Tone::Warn),
                "bad" => Some(Tone::Bad),
                "muted" => Some(Tone::Muted),
                _ => None,
            }),
        }
    }
}
