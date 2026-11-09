//! Checks for the `models` and `connections` pages (a copy of the real core rules: `settings_models.rs`, `gateway/routing.rs`,
//! `mcp/spec.rs`; `fake-agentd` does not depend on `agentd`).

use serde_json::{Map, Value};

/// Field error: path and text.
pub type FieldErr = (String, String);

fn err(field: impl Into<String>, msg: &str) -> FieldErr {
    (field.into(), msg.to_string())
}

/// `provider.<name>` or `account.<id>`.
pub fn valid_secret_ref(r: &str) -> bool {
    let Some((kind, rest)) = r.split_once('.') else {
        return false;
    };
    match kind {
        "provider" => {
            !rest.is_empty()
                && rest.len() <= 32
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        }
        "account" => {
            !rest.is_empty()
                && rest.len() <= 18
                && rest.bytes().all(|b| b.is_ascii_digit())
                && !rest.starts_with('0')
        }
        _ => false,
    }
}

fn valid_base_url(s: &str) -> bool {
    let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && s.len() <= 2048
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn model_account(id: &str, v: &Value, out: &mut Vec<FieldErr>) {
    let path = format!("accounts.{id}");
    if !id.parse::<i64>().is_ok_and(|n| n > 0) || id.starts_with('0') || id.starts_with('+') {
        out.push(err(path.clone(), "the key is an account number"));
    }
    let Some(o) = v.as_object() else {
        out.push(err(path, "object expected"));
        return;
    };
    for (k, val) in o {
        let f = format!("{path}.{k}");
        match k.as_str() {
            "base_url" if !val.as_str().is_some_and(valid_base_url) => out.push(err(
                f,
                "an http:// or https:// address without login and password is required",
            )),
            "model"
                if !val
                    .as_str()
                    .is_some_and(|m| !m.trim().is_empty() && m.len() <= 200) =>
            {
                out.push(err(
                    f,
                    "a non-empty string up to 200 characters is required",
                ));
            }
            "secret_ref" if !val.as_str().is_some_and(valid_secret_ref) => out.push(err(
                f,
                "a reference like account.<id> or provider.<name> is required, not a value",
            )),
            "base_url" | "model" | "secret_ref" => {}
            _ => out.push(err(f, "unknown field")),
        }
    }
}

fn price(model: &str, v: &Value, out: &mut Vec<FieldErr>) {
    let path = format!("prices.{model}");
    let Some(o) = v.as_object() else {
        out.push(err(path, "object expected"));
        return;
    };
    for k in ["input_per_m", "output_per_m"] {
        if !o
            .get(k)
            .and_then(Value::as_f64)
            .is_some_and(|n| n.is_finite() && n >= 0.0)
        {
            out.push(err(
                format!("{path}.{k}"),
                "a number not less than 0 is required",
            ));
        }
    }
    if let Some(c) = o.get("currency")
        && !c
            .as_str()
            .is_some_and(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
    {
        out.push(err(
            format!("{path}.currency"),
            "three uppercase Latin letters",
        ));
    }
    for k in o
        .keys()
        .filter(|k| !["input_per_m", "output_per_m", "currency"].contains(&k.as_str()))
    {
        out.push(err(format!("{path}.{k}"), "unknown field"));
    }
}

/// Page `models`: `accounts`, `prices`, `context_budget`.
pub fn models(obj: &Map<String, Value>) -> Vec<FieldErr> {
    let mut out = Vec::new();
    for k in obj
        .keys()
        .filter(|k| !["accounts", "prices", "context_budget"].contains(&k.as_str()))
    {
        out.push(err(k.clone(), "unknown field"));
    }
    match obj.get("accounts") {
        None => {}
        Some(Value::Object(m)) => m.iter().for_each(|(id, v)| model_account(id, v, &mut out)),
        Some(_) => out.push(err("accounts", "object expected")),
    }
    match obj.get("prices") {
        None => {}
        Some(Value::Object(m)) => m.iter().for_each(|(k, v)| price(k, v, &mut out)),
        Some(_) => out.push(err("prices", "object expected")),
    }
    if let Some(b) = obj.get("context_budget")
        && !b.as_u64().is_some_and(|n| (256..=4_000_000).contains(&n))
    {
        out.push(err("context_budget", "integer from 256 to 4000000 tokens"));
    }
    out
}

fn list<'a>(doc: &'a Map<String, Value>, key: &str, errs: &mut Vec<FieldErr>) -> &'a [Value] {
    match doc.get(key) {
        None => &[],
        Some(Value::Array(a)) => a,
        Some(_) => {
            errs.push(err(key, "array expected"));
            &[]
        }
    }
}

fn valid_mcp_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && !n.contains("__")
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn mcp_server(i: usize, v: &Value, errs: &mut Vec<FieldErr>) {
    let f = |n: &str| format!("mcp[{i}].{n}");
    if !valid_mcp_name(v["name"].as_str().unwrap_or_default()) {
        errs.push(err(
            f("name"),
            "1 to 32 characters a-z, 0-9, \"-\" and \"_\", without \"__\"",
        ));
    }
    match v["transport"].as_str() {
        Some("stdio") if v["command"].as_str().is_none_or(|c| c.trim().is_empty()) => {
            errs.push(err(f("command"), "a command is required"));
        }
        Some("stdio") => {}
        Some("http") => {
            let url = v["url"].as_str().unwrap_or_default().trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() < 9 {
                errs.push(err(f("url"), "an http or https address"));
            }
        }
        _ => errs.push(err(f("transport"), "stdio or http")),
    }
    if v.get("enabled")
        .is_some_and(|e| !e.is_boolean() && !e.is_null())
    {
        errs.push(err(f("enabled"), "true or false"));
    }
    if v.get("timeout_secs")
        .is_some_and(|t| !t.is_null() && !t.as_u64().is_some_and(|s| (1..=300).contains(&s)))
    {
        errs.push(err(f("timeout_secs"), "integer from 1 to 300"));
    }
    for k in v.as_object().into_iter().flat_map(Map::keys) {
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
            errs.push(err(f(k), "unknown field"));
        }
    }
}

/// Page `connections`: `contacts`, `routes`, `mcp`.
pub fn connections(doc: &Map<String, Value>) -> Vec<FieldErr> {
    let mut errs = Vec::new();
    for k in doc
        .keys()
        .filter(|k| !["contacts", "routes", "mcp"].contains(&k.as_str()))
    {
        errs.push(err(k.clone(), "unknown field"));
    }
    let mut names = Vec::new();
    for (i, c) in list(doc, "contacts", &mut errs).iter().enumerate() {
        match c["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.chars().count() <= 80)
        {
            Some(n) => names.push(n.to_string()),
            None => errs.push(err(
                format!("contacts[{i}].name"),
                "string of 1 to 80 characters",
            )),
        }
        let ok = c["handles"].as_object().is_some_and(|h| {
            h.values()
                .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
        });
        if !ok {
            errs.push(err(
                format!("contacts[{i}].handles"),
                "object channel -> non-empty string",
            ));
        }
    }
    for (i, r) in list(doc, "routes", &mut errs).iter().enumerate() {
        if !r["contact"]
            .as_str()
            .is_some_and(|n| names.iter().any(|x| x == n))
        {
            errs.push(err(
                format!("routes[{i}].contact"),
                "a contact name from contacts is required",
            ));
        }
        if r["session_id"].as_i64().filter(|n| *n > 0).is_none() {
            errs.push(err(format!("routes[{i}].session_id"), "positive integer"));
        }
    }
    match doc.get("mcp") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            let mut seen: Vec<&str> = Vec::new();
            for (i, v) in a.iter().enumerate() {
                mcp_server(i, v, &mut errs);
                let name = v["name"].as_str().unwrap_or_default();
                if seen.contains(&name) {
                    errs.push(err(format!("mcp[{i}].name"), "name is already taken"));
                }
                seen.push(name);
            }
        }
        Some(_) => errs.push(err("mcp", "array expected")),
    }
    errs
}
