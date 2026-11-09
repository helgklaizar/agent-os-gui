//! Parsing of persona and account bodies (admin): value sets, lengths, account type immutability.

use serde_json::Value;

use super::input::{AccountInput, PersonaInput};
use super::parse::{Body, Parsed};

pub fn parse_persona(v: &Value, create: bool) -> Parsed<PersonaInput> {
    let mut b = Body::new(
        v,
        &[
            "name",
            "color",
            "model_acc",
            "style",
            "lang",
            "signature",
            "mem_mode",
            "notif_mode",
            "work_hours",
            "quiet_hours",
            "account_ids",
        ],
    )?;
    if create {
        b.require("name");
        b.require("color");
    }
    let account_ids = match b.obj.get("account_ids") {
        None => None,
        Some(Value::Array(a)) => {
            let ids: Vec<i64> = a
                .iter()
                .filter_map(Value::as_i64)
                .filter(|n| *n > 0)
                .collect();
            if ids.len() == a.len() {
                let mut sorted = ids;
                sorted.sort_unstable();
                sorted.dedup();
                Some(sorted)
            } else {
                b.bad("account_ids", "an array of positive integers is required");
                None
            }
        }
        Some(_) => {
            b.bad("account_ids", "an array of positive integers is required");
            None
        }
    };
    let out = PersonaInput {
        name: b.text("name", 1, 100),
        color: b.text("color", 1, 64),
        model_acc: b.id_or_null("model_acc"),
        style: b.text("style", 0, 4000),
        lang: b.text("lang", 1, 16),
        signature: b.text("signature", 0, 1000),
        mem_mode: b.one_of("mem_mode", &["own", "shared", "none"]),
        notif_mode: b.one_of("notif_mode", &["all", "important", "digest", "off"]),
        work_hours: b.text("work_hours", 0, 64),
        quiet_hours: b.text("quiet_hours", 0, 64),
        account_ids,
    };
    b.finish(out)
}

pub fn parse_account(v: &Value, create: bool) -> Parsed<AccountInput> {
    let mut b = Body::new(v, &["type", "label"])?;
    if create {
        b.require("type");
        b.require("label");
    }
    let kind = b.text("type", 1, 32);
    if let Some(k) = &kind
        && !k
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        b.bad("type", "allowed: a-z, 0-9, \"_\" and \"-\"");
    }
    if !create && b.obj.contains_key("type") {
        b.errs.retain(|e| e.field != "type");
        b.bad("type", "the account type cannot be changed");
    }
    let out = AccountInput {
        kind,
        label: b.text("label", 1, 100),
    };
    b.finish(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fields<T: std::fmt::Debug>(r: Parsed<T>) -> Vec<String> {
        r.unwrap_err().into_iter().map(|e| e.field).collect()
    }

    #[test]
    fn account_type_charset_and_immutability() {
        assert_eq!(
            fields(parse_account(
                &json!({"type": "Bad Type", "label": "x"}),
                true
            )),
            ["type"]
        );
        assert_eq!(
            fields(parse_account(&json!({"type": "wa"}), false)),
            ["type"]
        );
        assert!(parse_account(&json!({"label": "x"}), false).is_ok());
    }
}
