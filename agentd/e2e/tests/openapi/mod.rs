//! Parser for `openapi-draft.yaml` used by contract tests: object schemas and operation response codes.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub const YAML: &str = include_str!("../../../../docs/reference/openapi-draft.yaml");

#[derive(Default, Debug)]
pub struct Schema {
    pub required: BTreeSet<String>,
    pub props: BTreeMap<String, Vec<String>>,
}

/// Types from `{ type: integer }`, `{ type: [integer, 'null'] }` and `{ $ref: … }` (object).
pub fn types_of(spec: &str) -> Vec<String> {
    // `oneOf: [ $ref, {type: 'null'} ]` - an object or null.
    if spec.contains("$ref") {
        // References to string-enum schemas (`PermissionAction`, `PermissionMode`, `ModulePlacement`) are strings, all others are objects.
        let enum_ref = ["PermissionAction", "PermissionMode", "ModulePlacement"]
            .iter()
            .any(|n| spec.contains(&format!("schemas/{n}")));
        let mut t = vec![if enum_ref { "string" } else { "object" }.to_string()];
        if spec.contains("'null'") {
            t.push("null".into());
        }
        return t;
    }
    let Some(i) = spec.find("type:") else {
        return vec!["any".into()];
    };
    let rest = spec[i + 5..].trim_start();
    let raw = match rest.strip_prefix('[') {
        Some(r) => r.split(']').next().unwrap_or(""),
        None => rest.split([',', '}']).next().unwrap_or(""),
    };
    raw.split(',')
        .map(|t| t.trim().trim_matches('\'').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

pub fn schemas() -> BTreeMap<String, Schema> {
    let mut out = BTreeMap::new();
    let mut in_schemas = false;
    let mut cur: Option<String> = None;
    let mut in_props = false;
    for line in YAML.lines() {
        if line.starts_with("  schemas:") {
            in_schemas = true;
            continue;
        }
        if !in_schemas {
            continue;
        }
        if let Some(name) = line
            .strip_prefix("    ")
            .and_then(|l| l.strip_suffix(':'))
            .filter(|n| !n.starts_with(' '))
        {
            cur = Some(name.to_string());
            in_props = false;
            out.insert(name.to_string(), Schema::default());
        } else if let Some(s) = cur.as_ref().and_then(|c| out.get_mut(c)) {
            if let Some(req) = line.strip_prefix("      pub required: [") {
                s.required = req
                    .trim_end_matches(']')
                    .split(',')
                    .map(|r| r.trim().to_string())
                    .collect();
            } else if line == "      properties:" {
                in_props = true;
            } else if in_props && line.starts_with("        ") && !line.starts_with("         ") {
                if let Some((k, spec)) = line.trim().split_once(':') {
                    s.props.insert(k.to_string(), types_of(spec));
                }
            } else if !line.starts_with("        ") {
                in_props = line == "      properties:";
            }
        }
    }
    out
}

pub fn type_ok(v: &Value, allowed: &[String]) -> bool {
    allowed.iter().any(|t| match t.as_str() {
        "integer" => v.is_i64() || v.is_u64(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "number" => v.is_number(),
        _ => true,
    })
}

pub fn check(name: &str, v: &Value, all: &BTreeMap<String, Schema>) {
    let s = all.get(name).unwrap_or_else(|| panic!("no schema {name}"));
    let obj = v
        .as_object()
        .unwrap_or_else(|| panic!("{name}: not an object: {v}"));
    for k in &s.required {
        assert!(
            obj.contains_key(k),
            "{name}: missing required field {k}: {v}"
        );
    }
    for (k, val) in obj {
        let types = s
            .props
            .get(k)
            .unwrap_or_else(|| panic!("{name}: field {k} is not described in the schema: {v}"));
        assert!(
            type_ok(val, types),
            "{name}.{k}: {val} is not one of {types:?}"
        );
    }
}

/// Response codes of an operation from the specification.
pub fn declared(path: &str, method: &str) -> BTreeSet<u16> {
    let (mut in_path, mut in_op, mut codes) = (false, false, BTreeSet::new());
    for line in YAML.lines() {
        if line.starts_with("components:") {
            break;
        }
        if let Some(p) = line.strip_prefix("  /").and_then(|p| p.strip_suffix(':')) {
            in_path = format!("/{p}") == path;
            in_op = false;
        } else if in_path
            && let Some(m) = line
                .strip_prefix("    ")
                .and_then(|m| m.strip_suffix(':'))
                .filter(|m| !m.starts_with(' '))
        {
            in_op = m == method;
        } else if in_path
            && in_op
            && let Some(c) = line.trim().strip_prefix('\'').and_then(|c| c.get(..3))
        {
            codes.extend(c.parse::<u16>());
        }
    }
    codes
}
