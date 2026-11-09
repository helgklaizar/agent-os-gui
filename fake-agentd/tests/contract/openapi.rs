//! Reading `openapi-draft.yaml` and checking a JSON value against a schema from `components.schemas`.

use std::path::PathBuf;

use serde_json::Value;

use crate::yaml::{self, Y};

pub struct Spec {
    pub root: Y,
}

pub fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/openapi-draft.yaml")
}

pub const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

impl Spec {
    pub fn load() -> Spec {
        let path = spec_path();
        let src =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        match yaml::parse(&src) {
            Ok(root) => Spec { root },
            Err(e) => panic!("openapi-draft.yaml not parsed: {e}"),
        }
    }

    /// All operations: `(METHOD, /path without /v1, node)`.
    pub fn operations(&self) -> Vec<(String, String, &Y)> {
        let mut out = Vec::new();
        let Some(paths) = self.root.get("paths") else {
            return out;
        };
        for (path, item) in paths.entries() {
            for (key, op) in item.entries() {
                if METHODS.contains(&key.as_str()) {
                    out.push((key.to_uppercase(), path.clone(), op));
                }
            }
        }
        out
    }

    pub fn op(&self, method: &str, path: &str) -> Option<&Y> {
        self.root.path(&["paths", path, &method.to_lowercase()])
    }

    pub fn statuses(&self, method: &str, path: &str) -> Vec<u16> {
        self.op(method, path)
            .and_then(|o| o.get("responses"))
            .map(|r| {
                r.entries()
                    .iter()
                    .filter_map(|(k, _)| k.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn scope(&self, method: &str, path: &str) -> Option<&str> {
        self.op(method, path)?.get("x-scope")?.as_str()
    }

    pub fn local_only(&self, method: &str, path: &str) -> bool {
        self.op(method, path)
            .and_then(|o| o.get("x-local-only"))
            .and_then(Y::as_str)
            == Some("true")
    }

    pub fn schema(&self, name: &str) -> &Y {
        self.root
            .path(&["components", "schemas", name])
            .unwrap_or_else(|| panic!("schema {name} is not in components.schemas"))
    }

    fn resolve<'a>(&'a self, schema: &'a Y) -> &'a Y {
        match schema.get("$ref").and_then(Y::as_str) {
            Some(r) => self.schema(r.rsplit('/').next().unwrap_or(r)),
            None => schema,
        }
    }

    /// Schema violations: an empty list means the value fits. Undeclared object fields count as a violation (the contract is exact).
    pub fn validate(&self, schema: &Y, v: &Value, at: &str) -> Vec<String> {
        let schema = self.resolve(schema);
        let mut errs = Vec::new();
        if let Some(all) = schema.get("allOf") {
            return self.validate_all_of(all.as_seq(), v, at);
        }
        if let Some(one) = schema.get("oneOf") {
            let ok = one
                .as_seq()
                .iter()
                .any(|s| self.validate(s, v, at).is_empty());
            if !ok {
                errs.push(format!("{at}: fits none of the oneOf variants"));
            }
            return errs;
        }
        if let Some(t) = schema.get("type") {
            let types: Vec<&str> = match t {
                Y::Str(s) => vec![s.as_str()],
                other => other.strs(),
            };
            if !types.iter().any(|t| type_matches(t, v)) {
                errs.push(format!("{at}: type {v} is not one of {types:?}"));
                return errs;
            }
        }
        if let (Some(e), Some(s)) = (schema.get("enum"), v.as_str())
            && !e.strs().contains(&s)
        {
            errs.push(format!("{at}: value {s:?} outside enum {:?}", e.strs()));
        }
        if let Some(n) = v.as_f64() {
            if let Some(min) = schema
                .get("minimum")
                .and_then(Y::as_str)
                .and_then(|m| m.parse::<f64>().ok())
                && n < min
            {
                errs.push(format!("{at}: {n} is below minimum {min}"));
            }
            if let Some(max) = schema
                .get("maximum")
                .and_then(Y::as_str)
                .and_then(|m| m.parse::<f64>().ok())
                && n > max
            {
                errs.push(format!("{at}: {n} is above maximum {max}"));
            }
        }
        if let (Some("uuid"), Some(s)) = (schema.get("format").and_then(Y::as_str), v.as_str())
            && !is_uuid(s)
        {
            errs.push(format!("{at}: {s:?} is not a uuid"));
        }
        if let Value::Object(obj) = v {
            errs.extend(self.validate_object(schema, obj, at));
        }
        if let (Value::Array(items), Some(item_schema)) = (v, schema.get("items")) {
            for (i, item) in items.iter().enumerate() {
                errs.extend(self.validate(item_schema, item, &format!("{at}[{i}]")));
            }
        }
        errs
    }

    fn validate_object(
        &self,
        schema: &Y,
        obj: &serde_json::Map<String, Value>,
        at: &str,
    ) -> Vec<String> {
        let mut errs = Vec::new();
        for req in schema.get("required").map(Y::strs).unwrap_or_default() {
            if !obj.contains_key(req) {
                errs.push(format!("{at}: missing required field {req}"));
            }
        }
        let Some(props) = schema.get("properties") else {
            return errs;
        };
        let open = schema.get("additionalProperties").and_then(Y::as_str) == Some("true");
        for (k, val) in obj {
            match props.get(k) {
                Some(p) => errs.extend(self.validate(p, val, &format!("{at}.{k}"))),
                None if open => {}
                None => errs.push(format!("{at}: field {k} is not described in the contract")),
            }
        }
        errs
    }

    /// `allOf`: merge properties and required fields of all parts and check as one object.
    fn validate_all_of(&self, parts: &[Y], v: &Value, at: &str) -> Vec<String> {
        let Value::Object(obj) = v else {
            return vec![format!("{at}: allOf expects an object")];
        };
        let mut errs = Vec::new();
        let mut known: Vec<String> = Vec::new();
        for part in parts {
            let part = self.resolve(part);
            known.extend(
                part.get("properties")
                    .map(|p| {
                        p.entries()
                            .iter()
                            .map(|(k, _)| k.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
            let narrowed: serde_json::Map<String, Value> = obj
                .iter()
                .filter(|(k, _)| part.get("properties").is_some_and(|p| p.get(k).is_some()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mut with_required = narrowed;
            for req in part.get("required").map(Y::strs).unwrap_or_default() {
                if !obj.contains_key(req) {
                    errs.push(format!("{at}: missing required field {req}"));
                } else if let Some(x) = obj.get(req) {
                    with_required.insert(req.to_string(), x.clone());
                }
            }
            errs.extend(self.validate_object(part, &with_required, at));
        }
        for k in obj.keys() {
            if !known.contains(k) {
                errs.push(format!("{at}: field {k} is not described in the contract"));
            }
        }
        errs
    }
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}
