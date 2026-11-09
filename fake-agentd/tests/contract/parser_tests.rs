//! YAML parsing: the subset and a check of counters against an independent line count of the file.

use crate::openapi::{self, Spec};
use crate::yaml;

#[test]
fn contract_yaml_parser_handles_subset() {
    let y = yaml::parse(
        "a:\n  b: { c: 1, d: [x, 'y, z'], e: { f: g } }\n  list:\n    - one\n    - key: v\n      other: w\n  text: >\n    folded\n    lines\n  '200': { $ref: '#/x/y' }\n  /p/{id}:\n    k: null\n",
    );
    let Ok(y) = y else { panic!("{y:?}") };
    assert_eq!(
        y.path(&["a", "b", "c"]).and_then(yaml::Y::as_str),
        Some("1")
    );
    assert_eq!(
        y.path(&["a", "b", "d"]).map(yaml::Y::strs),
        Some(vec!["x", "y, z"])
    );
    assert_eq!(
        y.path(&["a", "b", "e", "f"]).and_then(yaml::Y::as_str),
        Some("g")
    );
    assert_eq!(y.path(&["a", "list"]).map(|l| l.as_seq().len()), Some(2));
    assert_eq!(
        y.path(&["a", "list"])
            .and_then(|l| l.as_seq().get(1))
            .and_then(|m| m.get("other"))
            .and_then(yaml::Y::as_str),
        Some("w")
    );
    assert_eq!(
        y.path(&["a", "text"]).and_then(yaml::Y::as_str),
        Some("folded lines")
    );
    assert_eq!(
        y.path(&["a", "200", "$ref"]).and_then(yaml::Y::as_str),
        Some("#/x/y")
    );
    assert_eq!(y.path(&["a", "/p/{id}", "k"]), Some(&yaml::Y::Null));
}

#[test]
fn contract_yaml_counts_match_independent_line_count() {
    let src = std::fs::read_to_string(openapi::spec_path()).unwrap_or_else(|e| panic!("{e}"));
    let before_components: Vec<&str> = src
        .lines()
        .take_while(|l| !l.starts_with("components:"))
        .collect();
    let path_lines = before_components
        .iter()
        .filter(|l| l.starts_with("  /") && l.ends_with(':'))
        .count();
    let op_lines = before_components
        .iter()
        .filter(|l| openapi::METHODS.iter().any(|m| **l == format!("    {m}:")))
        .count();
    let after: Vec<&str> = src
        .lines()
        .skip_while(|l| *l != "  schemas:")
        .skip(1)
        .collect();
    let schema_lines = after
        .iter()
        .filter(|l| l.starts_with("    ") && !l.starts_with("     ") && l.ends_with(':'))
        .count();

    let spec = Spec::load();
    let paths = spec
        .root
        .get("paths")
        .map(|p| p.entries().len())
        .unwrap_or(0);
    let schemas = spec
        .root
        .path(&["components", "schemas"])
        .map(|p| p.entries().len())
        .unwrap_or(0);
    assert!(
        path_lines >= 50,
        "fewer than 50 paths in the file: {path_lines}"
    );
    assert_eq!(paths, path_lines, "number of paths");
    assert_eq!(spec.operations().len(), op_lines, "number of operations");
    assert_eq!(schemas, schema_lines, "number of schemas");
    assert_eq!(
        spec.root.get("openapi").and_then(yaml::Y::as_str),
        Some("3.1.0")
    );
}
