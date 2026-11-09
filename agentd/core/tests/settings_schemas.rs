//! Every settings page has a schema file `agentd/api/schemas/settings/<page>.json`; `about` and `security`
//! are marked `readOnly`, the rest are closed (`additionalProperties: false`).

use std::path::PathBuf;

const PAGES: [&str; 11] = [
    "account",
    "about",
    "personas",
    "models",
    "connections",
    "permissions",
    "security",
    "schedule",
    "modules",
    "tunnels",
    "memory",
];

fn load(page: &str) -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../api/schemas/settings")
        .join(format!("{page}.json"));
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn settings_page_schema_files_exist_for_all_pages() {
    for page in PAGES {
        let s = load(page);
        assert_eq!(s["type"], "object", "{page}");
        assert!(s["properties"].is_object(), "{page}: properties required");
    }
}

#[test]
fn settings_page_schemas_are_closed_and_read_only_marked() {
    for page in PAGES {
        let s = load(page);
        assert_eq!(s["additionalProperties"], false, "{page}: closed schema");
        let ro = matches!(page, "about" | "security");
        assert_eq!(s["readOnly"] == true, ro, "{page}: readOnly");
    }
}

#[test]
fn settings_page_schemas_declare_no_secret_fields() {
    for page in PAGES {
        let text = load(page).to_string().to_lowercase();
        for w in ["password", "api_key", "private_key"] {
            assert!(
                !text.contains(w),
                "{page}: the schema has a secret field {w}"
            );
        }
    }
}
