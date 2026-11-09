//! Observer rules from TOML: parsing and matching (the format is in the header of `rules/dangerous_cmd.toml`).

use serde::Deserialize;

use super::shell::{self, Segment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    /// Rules file not read (file name).
    Unreadable(String),
    /// TOML not parsed or a rule is unusable (description without rule text).
    Invalid(String),
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawRule {
    name: String,
    severity: String,
    verdict: String,
    #[serde(default)]
    program: Vec<String>,
    #[serde(default)]
    program_prefix: Vec<String>,
    #[serde(default)]
    flags_all: Vec<String>,
    #[serde(default)]
    args_any: Vec<String>,
    #[serde(default)]
    args_all: Vec<String>,
    #[serde(default)]
    arg_prefix_any: Vec<String>,
    #[serde(default)]
    redirect_prefix_any: Vec<String>,
    #[serde(default)]
    pipe_from: Vec<String>,
    #[serde(default)]
    pipe_to: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    rule: Vec<RawRule>,
}

/// Rule with validated fields.
#[derive(Debug)]
pub struct CmdRule {
    pub name: String,
    pub severity: &'static str,
    pub verdict: &'static str,
    raw: RawRule,
}

fn severity(s: &str) -> Option<&'static str> {
    ["low", "med", "high"].into_iter().find(|v| *v == s)
}

fn verdict(s: &str) -> Option<&'static str> {
    ["blocked", "asked"].into_iter().find(|v| *v == s)
}

/// Set of command rules.
#[derive(Debug)]
pub struct RuleSetFile {
    rules: Vec<CmdRule>,
}

fn starts(arg: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| arg.starts_with(p.as_str()))
}

fn segment_matches(r: &RawRule, s: &Segment) -> bool {
    (r.program.is_empty() || r.program.contains(&s.program))
        && (r.program_prefix.is_empty() || starts(&s.program, &r.program_prefix))
        && r.flags_all
            .iter()
            .all(|f| f.chars().all(|c| s.flags.contains(&c)))
        && (r.args_any.is_empty() || s.args.iter().any(|a| r.args_any.contains(a)))
        && r.args_all.iter().all(|a| s.args.contains(a))
        && (r.arg_prefix_any.is_empty() || s.args.iter().any(|a| starts(a, &r.arg_prefix_any)))
        && (r.redirect_prefix_any.is_empty()
            || s.redirects
                .iter()
                .any(|a| starts(a, &r.redirect_prefix_any)))
}

fn matches(r: &RawRule, segs: &[Segment]) -> bool {
    if !r.pipe_from.is_empty() {
        return segs.windows(2).any(|w| {
            w[0].piped_to_next
                && r.pipe_from.contains(&w[0].program)
                && r.pipe_to.contains(&w[1].program)
        });
    }
    segs.iter().any(|s| segment_matches(r, s))
}

impl RuleSetFile {
    pub fn parse(text: &str) -> Result<RuleSetFile, RuleError> {
        let file: RawFile =
            toml::from_str(text).map_err(|_| RuleError::Invalid("TOML not parsed".into()))?;
        let mut rules = Vec::new();
        for raw in file.rule {
            let bad = |why: &str| RuleError::Invalid(format!("rule {}: {why}", raw.name));
            let sev = severity(&raw.severity).ok_or_else(|| bad("severity"))?;
            let ver = verdict(&raw.verdict).ok_or_else(|| bad("verdict"))?;
            if raw.pipe_from.is_empty() != raw.pipe_to.is_empty() {
                return Err(bad("pipe_from and pipe_to must be set together"));
            }
            let has_condition = !(raw.program.is_empty()
                && raw.program_prefix.is_empty()
                && raw.flags_all.is_empty()
                && raw.args_any.is_empty()
                && raw.args_all.is_empty()
                && raw.arg_prefix_any.is_empty()
                && raw.redirect_prefix_any.is_empty()
                && raw.pipe_from.is_empty());
            if !has_condition {
                return Err(bad("no conditions: the rule would fire on any command"));
            }
            rules.push(CmdRule {
                name: raw.name.clone(),
                severity: sev,
                verdict: ver,
                raw,
            });
        }
        Ok(RuleSetFile { rules })
    }

    /// First rule that fires; "blocked" is stronger than "ask", so `blocked` is searched first.
    pub fn evaluate(&self, cmd: &str) -> Option<&CmdRule> {
        let segs = shell::parse(cmd);
        ["blocked", "asked"].into_iter().find_map(|v| {
            self.rules
                .iter()
                .find(|r| r.verdict == v && matches(&r.raw, &segs))
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMarkers {
    severity: String,
    markers: Vec<String>,
}

/// Injection markers.
#[derive(Debug)]
pub struct Markers {
    pub severity: &'static str,
    markers: Vec<String>,
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Markers {
    pub fn parse(text: &str) -> Result<Markers, RuleError> {
        let raw: RawMarkers =
            toml::from_str(text).map_err(|_| RuleError::Invalid("TOML not parsed".into()))?;
        let sev = severity(&raw.severity)
            .ok_or_else(|| RuleError::Invalid("markers: severity".into()))?;
        if raw.markers.iter().any(|m| normalize(m).is_empty()) {
            return Err(RuleError::Invalid("markers: empty marker".into()));
        }
        Ok(Markers {
            severity: sev,
            markers: raw.markers.iter().map(|m| normalize(m)).collect(),
        })
    }

    /// The marker that fired (the marker itself from the rules file, not the text under review).
    #[must_use]
    pub fn find(&self, text: &str) -> Option<String> {
        let t = normalize(text);
        self.markers
            .iter()
            .find(|m| t.contains(m.as_str()))
            .cloned()
    }
}
