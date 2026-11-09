//! The outbox queue survives a tab reload: kept in `sessionStorage` as JSON (only the message text,
//! session number, `client_id` and attachment ids; no token or other secrets in the entry). Parsing tolerates corruption:
//! a broken entry is dropped, extra fields are ignored, the entry count and text size are limited.

use serde_json::{Value, json};

/// Message in the outbox queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub session_id: i64,
    pub client_id: String,
    pub body_md: String,
    pub attachment_ids: Vec<i64>,
}

/// Limit of entries and text size per entry (bytes): tab storage is small.
pub const MAX_ITEMS: usize = 100;
pub const MAX_BODY: usize = 64 * 1024;

pub fn encode(list: &[Outgoing]) -> String {
    let items: Vec<Value> = list
        .iter()
        .take(MAX_ITEMS)
        .map(|o| {
            json!({"session_id": o.session_id, "client_id": o.client_id,
                   "body_md": o.body_md, "attachment_ids": o.attachment_ids})
        })
        .collect();
    Value::Array(items).to_string()
}

pub fn decode(text: &str) -> Vec<Outgoing> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out: Vec<Outgoing> = Vec::new();
    for v in items.iter().take(MAX_ITEMS) {
        let (Some(session_id), Some(client_id), Some(body)) = (
            v.get("session_id").and_then(Value::as_i64),
            v.get("client_id").and_then(Value::as_str),
            v.get("body_md").and_then(Value::as_str),
        ) else {
            continue;
        };
        if client_id.is_empty()
            || body.len() > MAX_BODY
            || out.iter().any(|o| o.client_id == client_id)
        {
            continue;
        }
        out.push(Outgoing {
            session_id,
            client_id: client_id.to_string(),
            body_md: body.to_string(),
            attachment_ids: v
                .get("attachment_ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(id: &str, body: &str) -> Outgoing {
        Outgoing {
            session_id: 7,
            client_id: id.into(),
            body_md: body.into(),
            attachment_ids: vec![3, 4],
        }
    }

    #[test]
    fn roundtrip_keeps_order_and_content() {
        let l = vec![o("a", "hello <b>x</b>"), o("b", "second")];
        assert_eq!(decode(&encode(&l)), l);
        assert!(decode(&encode(&[])).is_empty());
    }

    #[test]
    fn broken_input_is_dropped_not_fatal() {
        assert!(decode("not json").is_empty());
        assert!(decode("{}").is_empty());
        let d = decode(
            r#"[{"session_id":1,"client_id":"ok","body_md":"t"},{"session_id":"x","client_id":"a","body_md":"t"},
                {"session_id":1,"client_id":"","body_md":"t"},{"client_id":"n","body_md":"t"},
                {"session_id":1,"client_id":"ok","body_md":"duplicate"},{"session_id":2,"client_id":"z","body_md":"u","attachment_ids":[1,"x",2]}]"#,
        );
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[0].body_md, "t");
        assert_eq!(d[1].attachment_ids, vec![1, 2]);
        assert!(d[0].attachment_ids.is_empty());
    }

    #[test]
    fn limits_apply_both_ways() {
        let many: Vec<Outgoing> = (0..MAX_ITEMS + 20)
            .map(|i| o(&format!("c{i}"), "t"))
            .collect();
        assert_eq!(decode(&encode(&many)).len(), MAX_ITEMS);
        let big = Outgoing {
            body_md: "é".repeat(MAX_BODY),
            ..o("big", "")
        };
        assert!(
            decode(&encode(&[big, o("ok", "t")]))
                .iter()
                .all(|x| x.client_id == "ok")
        );
    }

    #[test]
    fn body_limit_is_inclusive() {
        let at = o("at", &"a".repeat(MAX_BODY));
        let over = o("over", &"a".repeat(MAX_BODY + 1));
        let got = decode(&encode(&[at, over]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].client_id, "at");
        // The limit is 64 KiB and not less: a 60 KiB message is saved.
        assert_eq!(decode(&encode(&[o("m", &"a".repeat(60 * 1024))])).len(), 1);
    }

    #[test]
    fn record_carries_no_token_field() {
        let e = encode(&[o("a", "t")]);
        for k in ["token", "authorization", "bearer"] {
            assert!(!e.to_lowercase().contains(k), "{e}");
        }
    }
}
