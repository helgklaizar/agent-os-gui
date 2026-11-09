//! Masking secrets before writing to `actions_log.args_json` and into responses (db.md section 3.7): values of keys like
//! token/key/secret/password/authorization and words with known token prefixes are replaced with `***`.

use serde_json::Value;

pub const MASK: &str = "***";

const KEY_PARTS: [&str; 7] = [
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "apikey",
    "credential",
];

const WORD_PREFIXES: [&str; 6] = ["sk-", "ghp_", "gho_", "xoxb-", "xoxp-", "AKIA"];

fn secret_key(key: &str) -> bool {
    let k = key.to_lowercase().replace(['-', '_'], "");
    KEY_PARTS.iter().any(|p| k.contains(p)) || k.ends_with("key")
}

/// Words of a string that look like a token (a prefix or the word after `Bearer`/`Basic`) are replaced with the mask.
pub fn mask_text(s: &str) -> String {
    let mut prev = "";
    let mut out = Vec::new();
    for word in s.split(' ') {
        let secret = WORD_PREFIXES.iter().any(|p| word.starts_with(p))
            || matches!(prev, "Bearer" | "Basic" | "bearer" | "basic");
        out.push(if secret && !word.is_empty() {
            MASK
        } else {
            word
        });
        prev = word;
    }
    out.join(" ")
}

pub fn mask_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(mask_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(mask_value).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    let masked = if secret_key(k) && !x.is_null() {
                        Value::String(MASK.to_string())
                    } else {
                        mask_value(x)
                    };
                    (k.clone(), masked)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}
