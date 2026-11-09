//! Request body parsing: types, lengths, value sets. Existence of related objects is checked by the store.

use serde_json::{Map, Value};

use super::input::{FolderInput, MAX_NAME_CHARS, MAX_PATH_CHARS, SessionInput};
use crate::FieldError;

pub(super) type Parsed<T> = Result<T, Vec<FieldError>>;

/// Per-field error accumulator.
pub(super) struct Body<'a> {
    pub(super) obj: &'a Map<String, Value>,
    pub(super) prefix: &'a str,
    pub(super) errs: Vec<FieldError>,
}

impl<'a> Body<'a> {
    pub(super) fn new(v: &'a Value, allowed: &[&str]) -> Result<Body<'a>, Vec<FieldError>> {
        let Some(obj) = v.as_object() else {
            return Err(vec![FieldError::new("", "a JSON object is expected")]);
        };
        let mut b = Body {
            obj,
            prefix: "",
            errs: Vec::new(),
        };
        for k in obj.keys().filter(|k| !allowed.contains(&k.as_str())) {
            b.errs.push(FieldError::new(k, "unknown field"));
        }
        Ok(b)
    }

    pub(super) fn bad(&mut self, field: &str, msg: &str) {
        self.errs
            .push(FieldError::new(&format!("{}{field}", self.prefix), msg));
    }

    pub(super) fn finish<T>(self, value: T) -> Parsed<T> {
        if self.errs.is_empty() {
            Ok(value)
        } else {
            Err(self.errs)
        }
    }

    /// String of `min..=max` characters without `\0`; empty after trimming only when `min = 0`.
    pub(super) fn text(&mut self, key: &str, min: usize, max: usize) -> Option<String> {
        let v = self.obj.get(key)?;
        let Some(s) = v.as_str() else {
            self.bad(key, "a string is required");
            return None;
        };
        let n = s.chars().count();
        if s.contains('\0') {
            self.bad(key, "null character is not allowed");
        } else if n > max {
            self.bad(key, &format!("at most {max} characters"));
        } else if n < min || (min > 0 && s.trim().is_empty()) {
            self.bad(key, "must not be empty");
        } else {
            return Some(s.to_string());
        }
        None
    }

    pub(super) fn require(&mut self, key: &str) {
        if !self.obj.contains_key(key) {
            self.bad(key, "required field");
        }
    }

    pub(super) fn id_or_null(&mut self, key: &str) -> Option<Option<i64>> {
        match self.obj.get(key)? {
            Value::Null => Some(None),
            v => match v.as_i64().filter(|n| *n > 0) {
                Some(n) => Some(Some(n)),
                None => {
                    self.bad(key, "a positive integer or null is required");
                    None
                }
            },
        }
    }

    pub(super) fn boolean(&mut self, key: &str) -> Option<bool> {
        let v = self.obj.get(key)?;
        if v.is_boolean() {
            return v.as_bool();
        }
        self.bad(key, "true or false is required");
        None
    }

    pub(super) fn one_of(&mut self, key: &str, set: &[&str]) -> Option<String> {
        let s = self.obj.get(key)?.as_str().filter(|s| set.contains(s));
        match s {
            Some(s) => Some(s.to_string()),
            None => {
                self.bad(key, &format!("allowed: {}", set.join(", ")));
                None
            }
        }
    }
}

/// Absolute path without `..` and `\0`, not longer than the limit. Symlink checks are on the file system side.
pub fn check_path(path: &str) -> Result<(), &'static str> {
    if path.chars().count() > MAX_PATH_CHARS {
        Err("path is longer than allowed")
    } else if path.contains('\0') {
        Err("null character is not allowed")
    } else if !path.starts_with('/') {
        Err("an absolute path is required")
    } else if path.split('/').any(|c| c == "..") {
        Err("\"..\" in a path is not allowed")
    } else {
        Ok(())
    }
}

pub fn parse_folder(v: &Value, create: bool) -> Parsed<FolderInput> {
    let mut b = Body::new(v, &["name", "parent_id", "persona_id", "open"])?;
    if create {
        b.require("name");
    }
    let out = FolderInput {
        name: b.text("name", 1, MAX_NAME_CHARS),
        parent_id: b.id_or_null("parent_id"),
        persona_id: b.id_or_null("persona_id"),
        open: b.boolean("open"),
    };
    b.finish(out)
}

fn parse_cfg(v: &Value, errs: &mut Vec<FieldError>) -> Option<Map<String, Value>> {
    let Some(obj) = v.as_object() else {
        errs.push(FieldError::new("cfg", "an object is expected"));
        return None;
    };
    let mut b = Body::new(v, &["model", "perms", "mem", "notif", "ctx", "auto"]).ok()?;
    b.prefix = "cfg.";
    for k in obj
        .keys()
        .filter(|k| !["model", "perms", "mem", "notif", "ctx", "auto"].contains(&k.as_str()))
    {
        // Unknown keys are already counted in `Body::new` without a prefix; here we rename to `cfg.<key>`.
        b.errs.retain(|e| e.field != *k);
        b.bad(k, "unknown field");
    }
    for (k, val) in obj.iter().filter(|(_, val)| !val.is_null()) {
        // The shape is "action — mode" pairs; weakening of higher-level rules is checked by the permissions engine on write.
        let (ok, msg) = match k.as_str() {
            "model" => (
                val.as_i64().is_some_and(|n| n > 0),
                "an account id or null is required",
            ),
            "perms" => (
                val.as_object().is_some_and(|m| {
                    m.iter().all(|(a, v)| {
                        crate::Action::parse(a).is_some()
                            && v.as_str().and_then(crate::Mode::parse).is_some()
                    })
                }),
                "an object \"action — allow|ask|deny\" is required",
            ),
            "mem" | "auto" => (val.is_boolean(), "true or false is required"),
            "notif" => (
                matches!(val.as_str(), Some("all" | "important" | "off")),
                "allowed: all, important, off",
            ),
            "ctx" => (
                matches!(val.as_str(), Some("parent" | "none")),
                "allowed: parent, none",
            ),
            _ => (true, ""),
        };
        if !ok {
            b.bad(k, msg);
        }
    }
    let cfg = obj.clone();
    errs.extend(b.errs);
    Some(cfg)
}

pub fn parse_session(v: &Value, create: bool) -> Parsed<SessionInput> {
    let allowed: &[&str] = if create {
        &["name", "folder_id", "persona_id", "path"]
    } else {
        &["name", "folder_id", "persona_id", "path", "cfg", "archived"]
    };
    let mut b = Body::new(v, allowed)?;
    if create {
        b.require("name");
    }
    let path = match b.obj.get("path") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => match check_path(s) {
            Ok(()) => Some(Some(s.clone())),
            Err(m) => {
                b.bad("path", m);
                None
            }
        },
        Some(_) => {
            b.bad("path", "a string or null is required");
            None
        }
    };
    let mut cfg = None;
    if let Some(c) = b.obj.get("cfg") {
        let mut errs = Vec::new();
        cfg = parse_cfg(c, &mut errs);
        b.errs.extend(errs);
    }
    let out = SessionInput {
        name: b.text("name", 1, MAX_NAME_CHARS),
        folder_id: b.id_or_null("folder_id"),
        persona_id: b.id_or_null("persona_id"),
        path,
        cfg,
        archived: b.boolean("archived"),
    };
    b.finish(out)
}

pub fn parse_thread(v: &Value) -> Parsed<String> {
    let mut b = Body::new(v, &["name"])?;
    b.require("name");
    let name = b.text("name", 1, MAX_NAME_CHARS);
    b.finish(name.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fields<T: std::fmt::Debug>(r: Parsed<T>) -> Vec<String> {
        r.unwrap_err().into_iter().map(|e| e.field).collect()
    }

    #[test]
    fn path_rules() {
        assert_eq!(check_path("/home/u/p"), Ok(()));
        for bad in ["", "rel", "../x", "/a/../b", "/a\0b"] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_path(&format!("/{}", "a".repeat(MAX_PATH_CHARS))).is_err());
        assert_eq!(
            check_path("/a..b/c"),
            Ok(()),
            "\"..\" inside a name is not a traversal"
        );
    }

    #[test]
    fn name_limits_count_chars_not_bytes() {
        let ok = "é".repeat(MAX_NAME_CHARS);
        assert!(parse_folder(&json!({"name": ok}), true).is_ok());
        assert_eq!(
            fields(parse_folder(&json!({"name": "é".repeat(201)}), true)),
            ["name"]
        );
        assert_eq!(
            fields(parse_folder(&json!({"name": " \t"}), true)),
            ["name"]
        );
    }

    #[test]
    fn patch_vs_create_and_null() {
        assert_eq!(fields(parse_folder(&json!({}), true)), ["name"]);
        let p = parse_folder(&json!({"parent_id": null}), false).unwrap();
        assert_eq!((p.name, p.parent_id), (None, Some(None)));
        assert_eq!(
            fields(parse_folder(&json!({"parent_id": 0}), false)),
            ["parent_id"]
        );
        assert_eq!(fields(parse_folder(&json!({"zzz": 1}), false)), ["zzz"]);
    }

    #[test]
    fn cfg_fields_are_prefixed() {
        let f = fields(parse_session(
            &json!({"cfg": {"mem": 1, "x": 1, "perms": {"a": "deny"}}}),
            false,
        ));
        assert_eq!(
            f.iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            ["cfg.mem", "cfg.perms", "cfg.x"].into_iter().collect()
        );
        assert!(parse_session(&json!({"cfg": {"perms": {}, "model": null}}), false).is_ok());
    }
}
