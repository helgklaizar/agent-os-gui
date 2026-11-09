//! Log masking tests.

use super::*;
use serde_json::json;

#[test]
fn secrets_masked_in_log_args() {
    let v = json!({
        "url": "https://example.test/",
        "token": "abc", "API_KEY": "k-1", "client_secret": "s", "Password": "p",
        "headers": {"Authorization": "Bearer abcdefghijkl"},
        "list": ["sk-live-0123456789ab", "plain text"],
        "note": "ghp_0123456789abcdef0123",
        "n": 5, "t": null, "token_null": null
    });
    let m = mask_args(&v);
    assert_eq!(m["url"], "https://example.test/");
    for k in ["token", "API_KEY", "client_secret", "Password"] {
        assert_eq!(m[k], MASK, "{k}");
    }
    assert_eq!(m["headers"]["Authorization"], MASK);
    assert_eq!(m["list"][0], MASK);
    assert_eq!(m["list"][1], "plain text");
    assert_eq!(m["note"], MASK);
    assert_eq!(
        (&m["n"], &m["t"], &m["token_null"]),
        (&json!(5), &Value::Null, &Value::Null)
    );
    let text = m.to_string();
    for leak in ["abc\"", "k-1", "abcdefghijkl", "sk-live", "ghp_"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    assert_eq!(mask_args(&json!("short")), "short");
    // Multibyte strings shorter than the prefix do not break masking.
    assert_eq!(mask_args(&json!("téléphone")), "téléphone");
}
