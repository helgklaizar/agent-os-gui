//! Module manifest: valid and invalid ones, three placements, an unknown field, a missing required field.
//!
//! The validation rules are already covered by `settings_manifest.rs`; here: parsing into types, the user-facing error and
//! agreement of the parser with the single source of rules `module_manifest::validate`.

use controllers::manifest::{
    MAX_BYTES, Manifest, ManifestError, Placement, manifest_from_value, parse_manifest,
};
use controllers::module_manifest::validate;
use serde_json::{Value, json};

fn valid(placement: &str) -> Value {
    json!({
        "id": "notes", "name": "Notes", "version": "1.0.0", "entry": "ui/index.html", "placement": placement,
        "permissions": ["GET /sessions", "POST /sessions/*/messages"], "network": ["api.example.com"],
        "fs": ["~/notes"], "questions": [{"key": "label"}]
    })
}

fn with(patch: Value) -> String {
    let mut m = valid("panel");
    if let (Value::Object(a), Value::Object(b)) = (&mut m, patch) {
        a.extend(b);
    }
    m.to_string()
}

fn without(field: &str) -> String {
    let mut m = valid("panel");
    if let Value::Object(o) = &mut m {
        o.remove(field);
    }
    m.to_string()
}

fn invalid(fields: &[&str]) -> Result<Manifest, ManifestError> {
    Err(ManifestError::Invalid(
        fields.iter().map(|f| (*f).to_string()).collect(),
    ))
}

#[test]
fn manifest_valid_all_three_placements() {
    for (text, want) in [
        ("panel", Placement::Panel),
        ("widget", Placement::Widget),
        ("button", Placement::Button),
    ] {
        let m =
            parse_manifest(&valid(text).to_string()).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(m.placement, want);
        assert_eq!(m.id, "notes");
        assert_eq!(m.name, "Notes");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.entry, "ui/index.html");
        assert_eq!(m.network, vec!["api.example.com".to_string()]);
        assert_eq!(m.fs, vec!["~/notes".to_string()]);
        assert_eq!(m.permissions.len(), 2);
        assert_eq!(m.questions.len(), 1);
    }
}

#[test]
fn manifest_optional_fields_default_to_empty() {
    let json =
        r#"{"id":"min","name":"Min","version":"0.1","entry":"index.html","placement":"widget"}"#;
    let m = parse_manifest(json).expect("minimal manifest");
    assert!(
        m.permissions.is_empty()
            && m.network.is_empty()
            && m.fs.is_empty()
            && m.questions.is_empty()
    );
}

#[test]
fn manifest_missing_required_field_is_named() {
    for f in ["id", "name", "version", "entry", "placement"] {
        assert_eq!(parse_manifest(&without(f)), invalid(&[f]), "field {f}");
    }
}

#[test]
fn manifest_unknown_field_is_named() {
    assert_eq!(
        parse_manifest(&with(json!({"sneaky": 1}))),
        invalid(&["sneaky"])
    );
}

#[test]
fn manifest_bad_placement_rejected() {
    for bad in ["sidebar", "PANEL", "", "panel "] {
        assert_eq!(
            parse_manifest(&with(json!({"placement": bad}))),
            invalid(&["placement"]),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse_manifest(&with(json!({"placement": 7}))),
        invalid(&["placement"])
    );
}

#[test]
fn manifest_garbage_input_does_not_panic() {
    assert_eq!(parse_manifest(""), Err(ManifestError::NotJson));
    assert_eq!(parse_manifest("{"), Err(ManifestError::NotJson));
    assert_eq!(parse_manifest("{not json"), Err(ManifestError::NotJson));
    for not_object in ["[]", "null", "5", "\"x\"", "true"] {
        assert_eq!(
            parse_manifest(not_object),
            Err(ManifestError::NotObject),
            "{not_object}"
        );
    }
    assert_eq!(parse_manifest(&with(json!({"id": 5}))), invalid(&["id"]));
}

#[test]
fn manifest_all_bad_fields_are_listed_in_check_order() {
    let m = with(json!({"id": "Bad", "placement": "x", "entry": "../x", "extra": 1}));
    assert_eq!(
        parse_manifest(&m),
        invalid(&["extra", "id", "entry", "placement"])
    );
}

#[test]
fn manifest_admin_permissions_and_bad_lists_are_rejected_by_index() {
    assert_eq!(
        parse_manifest(&with(
            json!({"permissions": ["GET /sessions", "POST /tokens"]})
        )),
        invalid(&["permissions[1]"])
    );
    assert_eq!(
        parse_manifest(&with(
            json!({"network": ["https://x.test"], "fs": ["/etc"]})
        )),
        invalid(&["network[0]", "fs[0]"])
    );
    let many: Vec<String> = (0..33).map(|i| format!("GET /x{i}")).collect();
    assert_eq!(
        parse_manifest(&with(json!({ "permissions": many }))),
        invalid(&["permissions"])
    );
}

#[test]
fn manifest_name_with_evil_text_is_kept_literally() {
    // a hostile string in the name is valid text: shown literally
    let evil = "<img src=x onerror=alert(1)>";
    let m = parse_manifest(&with(json!({ "name": evil }))).expect("text as text");
    assert_eq!(m.name, evil);
    assert_eq!(
        parse_manifest(&with(json!({"name": "<".repeat(100_000)}))),
        Err(ManifestError::TooLarge),
        "a huge manifest is rejected by size before parsing"
    );
    assert_eq!(
        parse_manifest(&with(json!({"name": "<".repeat(81)}))),
        invalid(&["name"])
    );
}

#[test]
fn manifest_size_limit_boundary() {
    let pad = |n: usize| {
        let base = with(json!({"name": "n"}));
        // pad with spaces after the JSON: the text is exactly n bytes long
        format!("{base}{}", " ".repeat(n - base.len()))
    };
    assert!(
        parse_manifest(&pad(MAX_BYTES)).is_ok(),
        "exactly the limit is accepted"
    );
    assert_eq!(
        parse_manifest(&pad(MAX_BYTES + 1)),
        Err(ManifestError::TooLarge)
    );
}

#[test]
fn manifest_parse_agrees_with_single_source_of_rules() {
    let cases = [
        json!({}),
        json!({"id": "A"}),
        json!({"entry": "/abs"}),
        json!({"permissions": ["FETCH /x"]}),
        json!({"version": ""}),
        json!({"questions": "x"}),
        json!({"unknown": true, "placement": "window"}),
    ];
    for patch in cases {
        let text = with(patch);
        let value: Value = serde_json::from_str(&text).expect("json");
        let bad = validate(&value);
        match parse_manifest(&text) {
            Ok(_) => assert!(
                bad.is_empty(),
                "{text}: parser accepted, validate rejected {bad:?}"
            ),
            Err(ManifestError::Invalid(fields)) => assert_eq!(fields, bad, "{text}"),
            Err(other) => panic!("{text}: {other:?}"),
        }
        assert_eq!(parse_manifest(&text), manifest_from_value(&value));
    }
}

#[test]
fn manifest_error_message_is_readable_and_has_no_foreign_text() {
    let errors = [
        ManifestError::NotJson,
        ManifestError::NotObject,
        ManifestError::TooLarge,
        ManifestError::Invalid(vec!["id".into(), "permissions[0]".into()]),
    ];
    let mut seen = Vec::new();
    for e in &errors {
        let text = e.message();
        assert!(!text.is_empty() && !text.contains('⟦'), "{e:?}: {text:?}");
        assert!(!seen.contains(&text), "texts of different errors differ");
        seen.push(text);
    }
    let text = errors[3].message();
    assert!(
        text.contains("id") && text.contains("permissions[0]"),
        "{text}"
    );
}
