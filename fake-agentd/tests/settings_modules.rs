//! Mock side of `/modules`: catalog, edit, removal; the manifest is checked against `schemas/module-manifest.json` and the core rules.

mod s7_common;

use s7_common::*;
use serde_json::{Value, json};

fn schema() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/module-manifest.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// Minimal schema check: `required`, `properties.*.type`/`enum`/`pattern`-like `maxLength`, `additionalProperties`.
fn schema_errors(schema: &Value, doc: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(obj) = doc.as_object() else {
        return vec!["not an object".into()];
    };
    let props = schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for req in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !obj.contains_key(req) {
            errs.push(format!("missing {req}"));
        }
    }
    for (k, v) in obj {
        let Some(p) = props.get(k) else {
            if schema["additionalProperties"] == json!(false) {
                errs.push(format!("extra field {k}"));
            }
            continue;
        };
        match p["type"].as_str() {
            Some("string") if !v.is_string() => errs.push(format!("{k}: not a string")),
            Some("array") if !v.is_array() => errs.push(format!("{k}: not an array")),
            _ => {}
        }
        if let (Some(en), Some(s)) = (p["enum"].as_array(), v.as_str())
            && !en.iter().any(|e| e == s)
        {
            errs.push(format!("{k}: outside enum"));
        }
        if let (Some(max), Some(s)) = (p["maxLength"].as_u64(), v.as_str())
            && s.chars().count() as u64 > max
        {
            errs.push(format!("{k}: longer than {max}"));
        }
    }
    errs
}

#[tokio::test(flavor = "multi_thread")]
async fn manifest_schema_matches_catalog_and_rejects_bad() {
    let sch = schema();
    assert_eq!(
        sch["additionalProperties"],
        json!(false),
        "unknown fields are forbidden"
    );
    let r = rig().await;
    let all = r.get("/v1/modules").await;
    let catalog: Vec<&Value> = all.as_array().into_iter().flatten().collect();
    assert_eq!(catalog.len(), 3, "mock catalog: {all}");
    for m in catalog {
        assert_eq!(
            schema_errors(&sch, &m["manifest"]),
            Vec::<String>::new(),
            "{m}"
        );
    }
    assert!(schema_errors(&sch, &manifest("ok")).is_empty());
    let mut bad = manifest("ok");
    bad["extra"] = json!(1);
    assert!(!schema_errors(&sch, &bad).is_empty());
    bad = manifest("ok");
    bad["placement"] = json!("sidebar");
    assert!(!schema_errors(&sch, &bad).is_empty());
    bad = manifest("ok");
    bad.as_object_mut().map(|o| o.remove("id"));
    assert!(!schema_errors(&sch, &bad).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_install_patch_delete_cycle() {
    let r = rig().await;
    let all = r.get("/v1/modules").await;
    let ids: Vec<&str> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .collect();
    for want in ["weather-card", "notes-panel", "quick-btn"] {
        assert!(ids.contains(&want), "{ids:?}");
    }
    assert!(
        all.as_array()
            .into_iter()
            .flatten()
            .all(|m| m["installed"] == false)
    );
    let w = r
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"catalog_id": "weather-card"})),
        )
        .await;
    assert_eq!(w.status, 201, "{}", text(&w));
    let m = w.json();
    assert_eq!(
        (
            m["id"].as_str(),
            m["installed"].clone(),
            m["enabled"].clone(),
            m["placement"].as_str()
        ),
        (
            Some("weather-card"),
            json!(true),
            json!(true),
            Some("widget")
        )
    );
    let dup = r
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"catalog_id": "weather-card"})),
        )
        .await;
    assert_eq!((dup.status, code(&dup)), (409, "conflict".into()));
    assert_eq!(
        dup.json()["error"]["details"]["reason"],
        "already_installed"
    );
    let nope = r
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"catalog_id": "no-such"})),
        )
        .await;
    assert_eq!(nope.status, 404);
    let p = r
        .call(
            "PATCH",
            "/v1/modules/weather-card",
            Some(json!({"enabled": false, "placement": "button", "config": {"city": "Haifa"}})),
        )
        .await;
    assert_eq!(p.status, 200, "{}", text(&p));
    assert_eq!(
        (
            p.json()["enabled"].clone(),
            p.json()["placement"].clone(),
            p.json()["config"]["city"].clone()
        ),
        (json!(false), json!("button"), json!("Haifa"))
    );
    for (body, field) in [
        (json!({"placement": "sidebar"}), "placement"),
        (json!({"enabled": "yes"}), "enabled"),
        (json!({"config": [1]}), "config"),
        (json!({"config": {"big": "x".repeat(17_000)}}), "config"),
    ] {
        let w = r
            .call("PATCH", "/v1/modules/weather-card", Some(body))
            .await;
        assert_eq!(w.status, 422, "{}", text(&w));
        assert_eq!(fields(&w), vec![field.to_string()]);
    }
    assert_eq!(
        r.call("PATCH", "/v1/modules/ghost", Some(json!({"enabled": true})))
            .await
            .status,
        404
    );
    let installed: Vec<Value> = r
        .get("/v1/modules")
        .await
        .as_array()
        .cloned()
        .unwrap_or_default();
    let me = installed
        .iter()
        .find(|m| m["id"] == "weather-card")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        (me["installed"].clone(), me["config"]["city"].clone()),
        (json!(true), json!("Haifa"))
    );
    assert_eq!(
        installed
            .iter()
            .filter(|m| m["id"] == "weather-card")
            .count(),
        1,
        "without a duplicate from the catalog"
    );
    assert_eq!(
        r.call("DELETE", "/v1/modules/weather-card", None)
            .await
            .status,
        204
    );
    let back = r.get("/v1/modules").await;
    let me = back
        .as_array()
        .and_then(|l| l.iter().find(|m| m["id"] == "weather-card"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(me["installed"], json!(false), "returned to the catalog");
    assert_eq!(
        r.call("DELETE", "/v1/modules/weather-card", None)
            .await
            .status,
        404
    );
}
