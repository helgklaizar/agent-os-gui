//! Settings entity fields: persona, module.

use controllers::settings_entities::{module_specs, persona_specs, show_entity};
use controllers::settings_model::CoreModel;
use serde_json::json;

#[test]
fn persona_account_toggle_reads_account_ids() {
    let m = CoreModel {
        personas: vec![json!({"id": 2, "name": "A", "lang": "en", "account_ids": [5]})],
        accounts: vec![
            json!({"id": 5, "type": "tg", "label": "T"}),
            json!({"id": 6, "type": "wa", "label": "W"}),
        ],
        ..CoreModel::default()
    };
    let specs = persona_specs("2", &m);
    let find = |p: &str| {
        specs
            .iter()
            .find(|s| s.path == p)
            .map(|s| show_entity(s, &m))
    };
    assert_eq!(find("acc:5").as_deref(), Some("true"));
    assert_eq!(find("acc:6").as_deref(), Some("false"));
    assert_eq!(find("lang").as_deref(), Some("en"));
    assert_eq!(find("mem_mode").as_deref(), Some("own"));
}

#[test]
fn module_questions_become_fields_without_bad_keys() {
    let module = json!({"id": "m1", "manifest": {"questions": ["city", {"key": "unit", "label": "U"}, "a:b", 5, {"x": 1}]}, "config": {"city": "X"}});
    let specs = module_specs(&module);
    let paths: Vec<_> = specs.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, ["placement", "cfg:city", "cfg:unit"]);
    let m = CoreModel {
        modules: vec![module],
        ..CoreModel::default()
    };
    assert_eq!(show_entity(&specs[1], &m), "X");
    assert_eq!(show_entity(&specs[2], &m), "");
}
