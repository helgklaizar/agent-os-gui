//! Parsing Settings field input into JSON and building the patch (JSON merge patch): limits are checked before sending to the core.

use serde_json::{Value, json};

use crate::i18n::tr;
use crate::settings_fields::{Spec, Ty};

/// Why the input was rejected; the text comes from `en.ftl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bad {
    Long,
    Choice,
    Int,
    Time,
}

impl Bad {
    pub fn text(self) -> String {
        match self {
            Bad::Long => tr("set-err-long"),
            Bad::Choice => tr("set-err-choice"),
            Bad::Int => tr("set-err-int"),
            Bad::Time => tr("set-err-time"),
        }
    }
}

/// Parse entered text into a document value; the error is a text key for the field.
pub fn parse(s: &Spec, text: &str) -> Result<Value, Bad> {
    let t = text.trim();
    match &s.ty {
        Ty::Text(max) => {
            if text.chars().count() > *max {
                Err(Bad::Long)
            } else {
                Ok(json!(text))
            }
        }
        Ty::Toggle => Ok(json!(t == "true")),
        Ty::Choice(options) => {
            if options.iter().any(|(v, _)| v == t) {
                Ok(json!(t))
            } else {
                Err(Bad::Choice)
            }
        }
        Ty::ChoiceId(options) => {
            if t.is_empty() {
                return Ok(Value::Null);
            }
            match t.parse::<i64>() {
                Ok(n) if options.iter().any(|(v, _)| *v == n.to_string()) => Ok(json!(n)),
                _ => Err(Bad::Choice),
            }
        }
        Ty::Int(lo, hi) => {
            if t.is_empty() {
                return Ok(Value::Null);
            }
            match t.parse::<i64>() {
                Ok(n) if (*lo..=*hi).contains(&n) => Ok(json!(n)),
                _ => Err(Bad::Int),
            }
        }
        Ty::Time => {
            if t.is_empty() {
                return Ok(Value::Null);
            }
            let ok = t.len() == 5
                && t.as_bytes()[2] == b':'
                && t[..2].parse::<u8>().is_ok_and(|h| h < 24)
                && t[3..].parse::<u8>().is_ok_and(|n| n < 60);
            if ok { Ok(json!(t)) } else { Err(Bad::Time) }
        }
    }
}

/// A nested patch (JSON merge patch) from a (path, value) pair.
pub fn nest(path: &str, value: Value) -> Value {
    path.rsplit('.')
        .fold(value, |inner, key| json!({ key: inner }))
}

/// Deep merge of nested patch objects.
pub fn merge(into: &mut Value, other: Value) {
    match (into, other) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(&k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        a.insert(k, v);
                    }
                }
            }
        }
        (slot, other) => *slot = other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fields::{none_and, spec};

    fn int() -> Spec {
        spec("d", "a.b", "x".into(), Ty::Int(1, 16), "")
    }

    #[test]
    fn int_parse_bounds() {
        let s = int();
        assert_eq!(parse(&s, "1"), Ok(json!(1)));
        assert_eq!(parse(&s, "16"), Ok(json!(16)));
        assert_eq!(parse(&s, "0"), Err(Bad::Int));
        assert_eq!(parse(&s, "17"), Err(Bad::Int));
        assert_eq!(parse(&s, "x"), Err(Bad::Int));
        assert_eq!(parse(&s, ""), Ok(Value::Null));
    }

    #[test]
    fn time_parse_boundaries() {
        let s = spec("d", "t", "x".into(), Ty::Time, "");
        for ok in ["00:00", "23:59", "09:05"] {
            assert_eq!(parse(&s, ok), Ok(json!(ok)));
        }
        for bad in ["24:00", "12:60", "1:05", "12-05", "ab:cd", "12:5"] {
            assert_eq!(parse(&s, bad), Err(Bad::Time), "{bad}");
        }
    }

    #[test]
    fn text_limit_counts_chars() {
        let s = spec("d", "n", "x".into(), Ty::Text(3), "");
        assert!(parse(&s, "\u{430}\u{431}\u{432}").is_ok());
        assert_eq!(parse(&s, "\u{430}\u{431}\u{432}\u{433}"), Err(Bad::Long));
    }

    #[test]
    fn choice_id_maps_empty_to_null() {
        let s = spec(
            "d",
            "p",
            "x".into(),
            Ty::ChoiceId(none_and(vec![("3".into(), "A".into())])),
            "",
        );
        assert_eq!(parse(&s, ""), Ok(Value::Null));
        assert_eq!(parse(&s, "3"), Ok(json!(3)));
        assert_eq!(parse(&s, "4"), Err(Bad::Choice));
        assert_eq!(parse(&s, "x"), Err(Bad::Choice));
    }

    #[test]
    fn nest_and_merge_build_one_patch() {
        let mut p = nest("balance.fallback", json!("wait"));
        merge(&mut p, nest("balance.max_parallel", json!(4)));
        merge(&mut p, nest("roles.chat", json!(2)));
        assert_eq!(
            p,
            json!({"balance": {"fallback": "wait", "max_parallel": 4}, "roles": {"chat": 2}})
        );
    }
}
