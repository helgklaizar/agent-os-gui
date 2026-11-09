//! MCP server descriptions on the `connections` settings page, key `mcp` (the format is implementation-defined, see docs/reference/api.md).
//! Server: `{name, transport: stdio|http, command, args, env, url, enabled, timeout_secs, tools: {<tool>: <action>}}`.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::{Action, FieldError};

/// Default call and connect timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Upper bound of the timeout, in seconds.
pub const MAX_TIMEOUT_SECS: u64 = 300;
/// Risk class of a tool with no entry in `tools`: the `send` action (default rule `ask`, dangerous category).
pub const DEFAULT_RISK: Action = Action::Send;
/// Tool name prefix for the model: `mcp__<server>__<tool>`.
pub const TOOL_PREFIX: &str = "mcp__";
/// Process environment variables with this prefix cannot be set: that is how core tokens and settings are named.
const RESERVED_ENV_PREFIX: &str = "AGENTD_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: String,
    pub transport: Transport,
    pub enabled: bool,
    pub timeout: Duration,
    pub tools: BTreeMap<String, Action>,
}

impl ServerSpec {
    /// Tool risk class.
    #[must_use]
    pub fn risk_of(&self, tool: &str) -> Action {
        self.tools.get(tool).copied().unwrap_or(DEFAULT_RISK)
    }
}

fn bad(field: String, msg: &str) -> FieldError {
    FieldError::new(&field, msg)
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && !n.contains("__")
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn valid_env_key(k: &str) -> bool {
    !k.is_empty()
        && !k.starts_with(RESERVED_ENV_PREFIX)
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !k.starts_with(|c: char| c.is_ascii_digit())
}

fn str_list(v: &Value) -> Option<Vec<String>> {
    v.as_array()?
        .iter()
        .map(|a| a.as_str().map(str::to_string))
        .collect()
}

fn parse_one(i: usize, v: &Value, errs: &mut Vec<FieldError>) -> Option<ServerSpec> {
    let f = |n: &str| format!("mcp[{i}].{n}");
    let before = errs.len();
    let name = v["name"].as_str().unwrap_or_default();
    if !valid_name(name) {
        errs.push(bad(
            f("name"),
            "1 to 32 characters of a-z, 0-9, '-' and '_', without '__'",
        ));
    }
    let transport = match v["transport"].as_str() {
        Some("stdio") => {
            let command = v["command"].as_str().unwrap_or_default().trim();
            if command.is_empty() {
                errs.push(bad(f("command"), "a command is required"));
            }
            let args = match v.get("args") {
                None | Some(Value::Null) => Some(Vec::new()),
                Some(a) => str_list(a),
            };
            if args.is_none() {
                errs.push(bad(f("args"), "list of strings"));
            }
            let mut env = BTreeMap::new();
            match v.get("env") {
                None | Some(Value::Null) => {}
                Some(Value::Object(m)) => {
                    for (k, val) in m {
                        match val.as_str() {
                            Some(s) if valid_env_key(k) => {
                                env.insert(k.clone(), s.to_string());
                            }
                            _ => errs.push(bad(
                                f("env"),
                                "name of letters, digits and '_', not AGENTD_*; the value is a string",
                            )),
                        }
                    }
                }
                Some(_) => errs.push(bad(f("env"), "object of name -> string")),
            }
            Transport::Stdio {
                command: command.to_string(),
                args: args.unwrap_or_default(),
                env,
            }
        }
        Some("http") => {
            let url = v["url"].as_str().unwrap_or_default().trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() < 9 {
                errs.push(bad(f("url"), "http or https address"));
            }
            Transport::Http {
                url: url.to_string(),
            }
        }
        _ => {
            errs.push(bad(f("transport"), "stdio or http"));
            return None;
        }
    };
    let enabled = match v.get("enabled") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            errs.push(bad(f("enabled"), "true or false"));
            true
        }
    };
    let secs = match v.get("timeout_secs") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_SECS,
        Some(t) => t
            .as_u64()
            .filter(|s| (1..=MAX_TIMEOUT_SECS).contains(s))
            .unwrap_or_else(|| {
                errs.push(bad(f("timeout_secs"), "integer from 1 to 300"));
                DEFAULT_TIMEOUT_SECS
            }),
    };
    let mut tools = BTreeMap::new();
    match v.get("tools") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            for (t, a) in m {
                match a.as_str().and_then(Action::parse) {
                    Some(act) => {
                        tools.insert(t.clone(), act);
                    }
                    None => errs.push(bad(f("tools"), "tool -> read, send, delete, ...")),
                }
            }
        }
        Some(_) => errs.push(bad(f("tools"), "object of tool -> action")),
    }
    for k in v.as_object()?.keys() {
        if ![
            "name",
            "transport",
            "command",
            "args",
            "env",
            "url",
            "enabled",
            "timeout_secs",
            "tools",
        ]
        .contains(&k.as_str())
        {
            errs.push(bad(f(k), "unknown field"));
        }
    }
    (errs.len() == before).then(|| ServerSpec {
        name: name.to_string(),
        transport,
        enabled,
        timeout: Duration::from_secs(secs),
        tools,
    })
}

/// Parsing and validation of the `mcp` list on the `connections` page; errors are per field.
pub fn parse_all(doc: &Value, errs: &mut Vec<FieldError>) -> Vec<ServerSpec> {
    let list = match doc.get("mcp") {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Array(a)) => a,
        Some(_) => {
            errs.push(bad("mcp".into(), "an array is expected"));
            return Vec::new();
        }
    };
    let mut out: Vec<ServerSpec> = Vec::new();
    for (i, v) in list.iter().enumerate() {
        if let Some(s) = parse_one(i, v, errs) {
            if out.iter().any(|o| o.name == s.name) {
                errs.push(bad(format!("mcp[{i}].name"), "name already taken"));
            } else {
                out.push(s);
            }
        }
    }
    out
}

/// Tool name for the model.
#[must_use]
pub fn tool_name(server: &str, tool: &str) -> String {
    format!("{TOOL_PREFIX}{server}__{tool}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_defaults() {
        let mut errs = Vec::new();
        let d = json!({"mcp": [{"name": "a", "transport": "stdio", "command": "/bin/x", "tools": {"t": "read"}}]});
        let l = parse_all(&d, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(l[0].timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        assert!(l[0].enabled);
        assert_eq!(l[0].risk_of("t"), Action::Read);
        assert_eq!(l[0].risk_of("other"), DEFAULT_RISK);
    }

    #[test]
    fn tool_name_roundtrip_prefix() {
        assert_eq!(tool_name("fake", "echo"), "mcp__fake__echo");
    }

    #[test]
    fn reserved_env_and_bad_keys_rejected() {
        for k in ["AGENTD_TOKEN", "1X", "a-b", ""] {
            assert!(!valid_env_key(k), "{k}");
        }
        assert!(valid_env_key("MCP_LOG"));
    }
}
