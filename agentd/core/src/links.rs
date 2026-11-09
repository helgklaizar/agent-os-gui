//! Browser link lists (`docs/reference/api.md` 5.8): session bookmarks and start page quick links.
//! Stored in `settings` under the keys `browser.bookmarks.<session_id>` and `browser.quicklinks`.

use serde_json::{Value, json};

use crate::FieldError;

/// No more than this many links in a list.
pub const MAX_LINKS: usize = 500;

/// Quick links key.
pub const QUICKLINKS_KEY: &str = "browser.quicklinks";

/// Session bookmarks key.
#[must_use]
pub fn bookmarks_key(session_id: i64) -> String {
    format!("browser.bookmarks.{session_id}")
}

/// Start page quick links until they are changed.
#[must_use]
pub fn default_quicklinks() -> Value {
    json!([
        {"title": "Google", "url": "https://www.google.com/"},
        {"title": "YouTube", "url": "https://www.youtube.com/"},
        {"title": "GitHub", "url": "https://github.com/"},
        {"title": "Wikipedia", "url": "https://www.wikipedia.org/"},
        {"title": "HackerNews", "url": "https://news.ycombinator.com/"},
    ])
}

fn link_ok(l: &Value) -> bool {
    let Some(o) = l.as_object() else { return false };
    if o.len() != 2 {
        return false;
    }
    let (Some(t), Some(u)) = (
        o.get("title").and_then(Value::as_str),
        o.get("url").and_then(Value::as_str),
    ) else {
        return false;
    };
    let lower = u.to_ascii_lowercase();
    !t.trim().is_empty()
        && t.chars().count() <= 200
        && u.len() <= 2048
        && (lower.starts_with("http://") || lower.starts_with("https://"))
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `PUT` body: an array of `{title, url}`; `title` 1…200 characters, `url` — `http`/`https` without spaces, up to 2048 characters.
#[must_use]
pub fn validate_links(body: &Value) -> Vec<FieldError> {
    let Some(list) = body.as_array() else {
        return vec![FieldError::new("body", "an array of links is expected")];
    };
    if list.len() > MAX_LINKS || !list.iter().all(link_ok) {
        return vec![FieldError::new(
            "body",
            "link: title and url (http or https), at most 500 links",
        )];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_rejects() {
        assert!(validate_links(&json!([])).is_empty());
        assert!(validate_links(&json!([{"title": "a", "url": "https://x.y/"}])).is_empty());
        for bad in [
            json!({}),
            json!([{"title": "", "url": "https://x.y/"}]),
            json!([{"title": "a", "url": "javascript:alert(1)"}]),
            json!([{"title": "a", "url": "https://x y"}]),
            json!([{"title": "a", "url": "https://x.y/", "extra": 1}]),
            json!([1]),
        ] {
            assert!(!validate_links(&bad).is_empty(), "{bad}");
        }
        let many: Vec<Value> = (0..=MAX_LINKS)
            .map(|_| json!({"title": "a", "url": "https://x.y/"}))
            .collect();
        assert!(!validate_links(&Value::Array(many)).is_empty());
    }

    #[test]
    fn keys() {
        assert_eq!(bookmarks_key(7), "browser.bookmarks.7");
    }
}
