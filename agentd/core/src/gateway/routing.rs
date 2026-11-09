//! Contacts and routing rules from the `connections` settings page (the format is implementation-defined, see docs/reference/api.md).
//! Document: `contacts: [{name, handles: {tg|wa|mail: string}, persona_id?}]`, `routes: [{contact, session_id}]`.

use serde_json::Value;

use crate::FieldError;

/// A contact from the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    pub handles_json: String,
    pub persona_id: Option<i64>,
}

/// Channel subtype from the account type: `tg`, `wa`, `mail` (mail: `gmail`, `mail`, `imap`); any other type stays as is.
#[must_use]
pub fn channel_of(account_type: &str) -> String {
    match account_type {
        "gmail" | "mail" | "imap" => "mail".to_string(),
        other => other.to_string(),
    }
}

fn same_handle(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The contact that has this `handle` recorded for the channel.
#[must_use]
pub fn match_contact(doc: &Value, channel: &str, handle: &str) -> Option<Contact> {
    doc["contacts"].as_array()?.iter().find_map(|c| {
        let h = c["handles"][channel].as_str()?;
        same_handle(h, handle).then(|| Contact {
            name: c["name"].as_str().unwrap_or_default().to_string(),
            handles_json: c["handles"].to_string(),
            persona_id: c["persona_id"].as_i64(),
        })
    })
}

/// The rule's session for a contact; no rule gives `None` (the shared feed).
#[must_use]
pub fn route_for(doc: &Value, contact: &str) -> Option<i64> {
    doc["routes"]
        .as_array()?
        .iter()
        .find(|r| r["contact"].as_str() == Some(contact))
        .and_then(|r| r["session_id"].as_i64())
}

fn bad(field: String, msg: &str) -> FieldError {
    FieldError::new(&field, msg)
}

/// Validation of the `connections` page; an empty list is a valid document.
#[must_use]
pub fn validate(doc: &serde_json::Map<String, Value>) -> Vec<FieldError> {
    let mut errs = Vec::new();
    for k in doc
        .keys()
        .filter(|k| !["contacts", "routes", "mcp"].contains(&k.as_str()))
    {
        errs.push(bad(k.clone(), "unknown field"));
    }
    let mut names = Vec::new();
    for (i, c) in list(doc, "contacts", &mut errs).iter().enumerate() {
        let f = |n: &str| format!("contacts[{i}].{n}");
        match c["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.chars().count() <= 80)
        {
            Some(n) => names.push(n.to_string()),
            None => errs.push(bad(f("name"), "string of 1 to 80 characters")),
        }
        match c["handles"].as_object() {
            Some(h)
                if h.values()
                    .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())) => {}
            _ => errs.push(bad(f("handles"), "object of channel -> non-empty string")),
        }
        if !c["persona_id"].is_null()
            && c.get("persona_id").is_some()
            && c["persona_id"].as_i64().is_none()
        {
            errs.push(bad(f("persona_id"), "integer or null"));
        }
    }
    for (i, r) in list(doc, "routes", &mut errs).iter().enumerate() {
        let f = |n: &str| format!("routes[{i}].{n}");
        if !r["contact"]
            .as_str()
            .is_some_and(|n| names.iter().any(|x| x == n))
        {
            errs.push(bad(
                f("contact"),
                "a contact name from contacts is required",
            ));
        }
        if r["session_id"].as_i64().filter(|n| *n > 0).is_none() {
            errs.push(bad(f("session_id"), "positive integer"));
        }
    }
    crate::mcp::spec::parse_all(&Value::Object(doc.clone()), &mut errs);
    errs
}

fn list<'a>(
    doc: &'a serde_json::Map<String, Value>,
    key: &str,
    errs: &mut Vec<FieldError>,
) -> &'a [Value] {
    match doc.get(key) {
        None => &[],
        Some(Value::Array(a)) => a,
        Some(_) => {
            errs.push(bad(key.to_string(), "an array is expected"));
            &[]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({"contacts": [{"name": "Vasya", "handles": {"tg": "@Vasya"}}],
               "routes": [{"contact": "Vasya", "session_id": 3}]})
    }

    #[test]
    fn matches_handle_ignoring_case_and_finds_route() {
        let c = match_contact(&doc(), "tg", " @vasya ").unwrap();
        assert_eq!(c.name, "Vasya");
        assert!(match_contact(&doc(), "wa", "@vasya").is_none());
        assert_eq!(route_for(&doc(), "Vasya"), Some(3));
        assert_eq!(route_for(&doc(), "Petya"), None);
        assert_eq!(channel_of("gmail"), "mail");
        assert_eq!(channel_of("tg"), "tg");
    }

    #[test]
    fn validation_reports_each_field() {
        let d = json!({"contacts": [{"name": "", "handles": "x"}], "routes": [{"contact": "None", "session_id": "a"}], "z": 1});
        let errs = validate(d.as_object().unwrap());
        assert_eq!(errs.len(), 5, "{errs:?}");
        assert!(validate(doc().as_object().unwrap()).is_empty());
    }
}
