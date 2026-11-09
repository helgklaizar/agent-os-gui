//! `contract_a1_routes`: A1 route responses against the schemas of `openapi-draft.yaml` (required fields, types, enums,
//! no fields outside the schema). Schemas are read from the file itself.

mod common;

use std::collections::BTreeMap;

use agentd_api::{Server, ServerConfig, Services};
use agentd_core::{Scope, TokenKind};
use common::{env, http_body, new_token};
use serde_json::{Value, json};

/// Object schema from `components/schemas`: field -> (type, enum), required fields.
#[derive(Debug, Default)]
struct Schema {
    props: BTreeMap<String, (String, Vec<String>)>,
    required: Vec<String>,
}

fn schemas() -> BTreeMap<String, Schema> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/openapi-draft.yaml"
    ))
    .unwrap();
    let mut out: BTreeMap<String, Schema> = BTreeMap::new();
    let mut in_schemas = false;
    let mut cur = String::new();
    for line in text.lines() {
        if line == "  schemas:" {
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
            cur = name.to_string();
            out.entry(cur.clone()).or_default();
            continue;
        }
        let Some(s) = out.get_mut(&cur) else { continue };
        if let Some(req) = line
            .strip_prefix("      required: [")
            .and_then(|l| l.strip_suffix(']'))
        {
            s.required = req.split(',').map(|x| x.trim().to_string()).collect();
        }
        // Top-level schema field: eight spaces, `name: { type: ..., enum: [...] }`.
        let field = line
            .strip_prefix("        ")
            .filter(|r| !r.starts_with(' ') && !r.starts_with('-'))
            .and_then(|r| r.split_once(':'));
        if let Some((name, def)) = field {
            let ty = def
                .split("type:")
                .nth(1)
                .map(|t| {
                    t.trim_start()
                        .trim_start_matches('[')
                        .split([',', ' ', '}', ']'])
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_default();
            let en = def
                .split("enum: [")
                .nth(1)
                .and_then(|e| e.split(']').next())
                .map(|e| e.split(',').map(|x| x.trim().to_string()).collect())
                .unwrap_or_default();
            s.props.insert(name.trim().to_string(), (ty, en));
        }
    }
    out
}

fn type_ok(v: &Value, ty: &str) -> bool {
    match ty {
        "integer" => v.is_i64() || v.is_u64(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "number" => v.is_number(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    }
}

/// Checks a value against a schema; `extra` are allowed fields from neighbouring schemas (`allOf`).
fn check(schemas: &BTreeMap<String, Schema>, name: &str, v: &Value, extra: &[&str]) {
    let s = schemas
        .get(name)
        .unwrap_or_else(|| panic!("no schema {name}"));
    let obj = v
        .as_object()
        .unwrap_or_else(|| panic!("{name}: not an object: {v}"));
    for r in &s.required {
        assert!(
            obj.contains_key(r),
            "{name}: missing required field {r} in {v}"
        );
    }
    for (k, val) in obj {
        if extra.contains(&k.as_str()) {
            continue;
        }
        let (ty, en) = s
            .props
            .get(k)
            .unwrap_or_else(|| panic!("{name}: field {k} is outside the schema"));
        // `null` is allowed where the schema permits it (type as a list or `oneOf`): only non-null values are checked.
        if !val.is_null() {
            assert!(type_ok(val, ty), "{name}.{k}: expected {ty}, got {val}");
            if !en.is_empty() {
                let got = val.as_str().unwrap_or_default();
                assert!(
                    en.iter().any(|e| e == got),
                    "{name}.{k}: {got} not in {en:?}"
                );
            }
        }
    }
}

#[test]
fn parser_reads_known_schema() {
    let s = schemas();
    let t = &s["Token"];
    assert!(t.required.contains(&"kind".to_string()));
    assert_eq!(
        t.props["kind"].1.len(),
        5,
        "kind enum: {:?}",
        t.props["kind"]
    );
    assert_eq!(t.props["id"].0, "integer");
    assert!(s["Health"].props.contains_key("capabilities"));
}

#[tokio::test]
async fn contract_a1_routes() {
    let schemas = schemas();
    let env = env();
    let admin = env
        .store
        .issue(new_token(TokenKind::Gui, Scope::Admin))
        .await
        .unwrap()
        .value
        .expose()
        .clone();
    let mut cfg = ServerConfig::new(env.dir.path().join("agentd.sock"));
    cfg.clock = env.clock.clone();
    let server = Server::start_booting(cfg).await.unwrap();
    let mut services = Services::new(env.store.clone());
    services.tokens = Some(env.store.clone());
    services.status = Some(env.db.clone());
    services.settings = Some(std::sync::Arc::new(agentd_db::SettingsRepo::new(
        env.db.clone(),
        env.clock.clone(),
    )));
    server.set_ready(services);
    let call = |m: &'static str, p: &'static str, body: Option<Value>, extra: Vec<String>| {
        let sock = server.socket_path().to_path_buf();
        let admin = admin.clone();
        async move {
            let mut s = tokio::net::UnixStream::connect(sock).await.unwrap();
            let mut hs = vec![format!("Authorization: Bearer {admin}")];
            let payload = body.map(|b| b.to_string()).unwrap_or_default();
            if !payload.is_empty() {
                hs.push(format!("Content-Length: {}", payload.len()));
            }
            hs.extend(extra);
            http_body(&mut s, m, p, "localhost", &hs, &payload)
                .await
                .unwrap()
        }
    };

    let h = call("GET", "/v1/health", None, vec![]).await;
    check(&schemas, "Health", &h.json(), &[]);
    check(
        &schemas,
        "Migration",
        &json!({"status": "running", "from": 0, "to": 1}),
        &[],
    );

    let created = call(
        "POST",
        "/v1/tokens",
        Some(json!({"name": "c", "scope": "chat", "ttl_days": 5})),
        vec![],
    )
    .await;
    assert_eq!(created.status, 201);
    assert!(
        created.json()["token"]
            .as_str()
            .is_some_and(|t| t.starts_with("agt_")),
        "TokenIssued.token"
    );
    check(&schemas, "Token", &created.json(), &["token"]);
    let list = call("GET", "/v1/tokens", None, vec![]).await;
    for t in list.json().as_array().unwrap() {
        check(&schemas, "Token", t, &[]);
    }

    let theme = call("GET", "/v1/settings/theme", None, vec![]).await;
    check(&schemas, "ThemePage", &theme.json(), &[]);
    let patched = call(
        "PATCH",
        "/v1/settings/theme",
        Some(json!({"theme": "dark"})),
        vec!["If-Match: 0".into()],
    )
    .await;
    check(&schemas, "ThemePage", &patched.json(), &[]);
    let page = call("GET", "/v1/settings/memory", None, vec![]).await;
    check(&schemas, "SettingsPage", &page.json(), &["source"]);

    // Errors: code from the Error schema enum, required fields present.
    let err = call("GET", "/v1/settings/nope", None, vec![]).await.json();
    let e = &err["error"];
    check(&schemas, "Error", &err, &[]);
    for k in ["code", "message", "details", "retryable", "request_id"] {
        assert!(e.get(k).is_some(), "{k}");
    }
    // Code is from the schema enum (the `enum: [bad_request, ...]` line of the `code` field).
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/openapi-draft.yaml"
    ))
    .unwrap();
    let enum_line = text
        .lines()
        .find(|l| l.contains("enum: [bad_request, unauthorized"))
        .expect("code enum");
    assert!(
        enum_line.contains(e["code"].as_str().unwrap()),
        "code {} not in the enum",
        e["code"]
    );
    server.shutdown().await;
}
