//! `models` settings page: provider accounts (`base_url`, `model`, `secret_ref`), model prices, context
//! budget. The page has no secret values: only the `secret_ref` reference.

use serde_json::{Map, Value};

use crate::{FieldError, valid_secret_ref};

const TOP: &[&str] = &["accounts", "prices", "context_budget"];
const PRICE: &[&str] = &["input_per_m", "output_per_m", "currency"];

fn err(field: String, msg: &str) -> FieldError {
    FieldError {
        field,
        message: msg.to_string(),
    }
}

/// `http(s)://host…` without a login and password in the address (credentials in a URL are a secret too).
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

fn account(id: &str, v: &Value, out: &mut Vec<FieldError>) {
    let path = format!("accounts.{id}");
    if !id.parse::<i64>().is_ok_and(|n| n > 0) || id.starts_with('0') || id.starts_with('+') {
        out.push(err(path.clone(), "the key is an account number"));
    }
    let Some(o) = v.as_object() else {
        out.push(err(path, "an object is expected"));
        return;
    };
    for (k, val) in o {
        let f = format!("{path}.{k}");
        match k.as_str() {
            "base_url" if !val.as_str().is_some_and(valid_base_url) => {
                out.push(err(
                    f,
                    "an http:// or https:// address without login and password is required",
                ));
            }
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
            "secret_ref" if !val.as_str().is_some_and(valid_secret_ref) => {
                out.push(err(
                    f,
                    "a reference like account.<id> or provider.<name> is required, not a value",
                ));
            }
            "base_url" | "model" | "secret_ref" => {}
            _ => out.push(err(f, "unknown field")),
        }
    }
}

fn price(model: &str, v: &Value, out: &mut Vec<FieldError>) {
    let path = format!("prices.{model}");
    let Some(o) = v.as_object() else {
        out.push(err(path, "an object is expected"));
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
                "a number of at least 0 is required",
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
    for k in o.keys().filter(|k| !PRICE.contains(&k.as_str())) {
        out.push(err(format!("{path}.{k}"), "unknown field"));
    }
}

pub(crate) fn validate(obj: &Map<String, Value>) -> Vec<FieldError> {
    let mut out = Vec::new();
    for k in obj.keys().filter(|k| !TOP.contains(&k.as_str())) {
        out.push(err(k.clone(), "unknown field"));
    }
    match obj.get("accounts") {
        None => {}
        Some(Value::Object(m)) => m.iter().for_each(|(id, v)| account(id, v, &mut out)),
        Some(_) => out.push(err("accounts".into(), "an object is expected")),
    }
    match obj.get("prices") {
        None => {}
        Some(Value::Object(m)) => m.iter().for_each(|(model, v)| price(model, v, &mut out)),
        Some(_) => out.push(err("prices".into(), "an object is expected")),
    }
    if let Some(b) = obj.get("context_budget")
        && !b.as_u64().is_some_and(|n| (256..=4_000_000).contains(&n))
    {
        out.push(err(
            "context_budget".into(),
            "an integer from 256 to 4000000 tokens",
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(v: Value) -> Vec<String> {
        validate(v.as_object().unwrap())
            .into_iter()
            .map(|e| e.field)
            .collect()
    }

    #[test]
    fn base_url_forms() {
        for ok in [
            "http://127.0.0.1:11434/v1",
            "https://api.example.test",
            "https://h/p?q=1",
        ] {
            assert!(valid_base_url(ok), "{ok}");
        }
        for bad in [
            "",
            "ftp://x",
            "https://",
            "https://u:p@h/",
            "https://h @x",
            "//h",
            "http:///x",
        ] {
            assert!(!valid_base_url(bad), "{bad}");
        }
    }

    #[test]
    fn page_rules() {
        assert!(fields(json!({})).is_empty());
        assert!(fields(json!({"accounts": {"3": {"model": "m", "secret_ref": "account.3"}}, "context_budget": 1000})).is_empty());
        assert_eq!(
            fields(json!({"accounts": {"3": {"api_key": "x"}}})),
            ["accounts.3.api_key"]
        );
        assert_eq!(fields(json!({"accounts": {"03": {}}})), ["accounts.03"]);
        assert_eq!(fields(json!({"context_budget": 255})), ["context_budget"]);
        assert!(fields(json!({"prices": {"m": {"input_per_m": 0, "output_per_m": 0}}})).is_empty());
    }
}
