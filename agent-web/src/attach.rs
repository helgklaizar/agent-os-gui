//! Chat attachments from the browser: file check before upload and parsing of the `POST /sessions/{id}/attachments` reply
//! (docs/reference/api.md section 5.2, 25 MiB limit). The client neither reads nor shows the file content.

use serde_json::Value;

/// Core limit per attachment (`MAX_ATTACHMENT_BYTES`), bytes.
pub const MAX_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub id: i64,
    pub name: String,
    pub size: u64,
}

/// The file is fit for upload: non-empty and within the limit; otherwise a reason for the user.
pub fn check_file(name: &str, size: u64) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("file has no name".to_string());
    }
    if size == 0 {
        return Err("file is empty".to_string());
    }
    if size > MAX_BYTES {
        return Err("file too large: limit 25 MiB".to_string());
    }
    Ok(())
}

/// Core `Attachment` reply (`id`, `name`, `size`); incomplete means `None`.
pub fn parse_uploaded(v: &Value) -> Option<Uploaded> {
    Some(Uploaded {
        id: v.get("id")?.as_i64()?,
        name: v.get("name")?.as_str()?.to_string(),
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Identifiers for the message `attachment_ids` field: the attachment path in the controller is the upload `id`.
pub fn attachment_ids(paths: &[String]) -> Vec<i64> {
    paths.iter().filter_map(|p| p.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn size_limits() {
        assert!(check_file("a.txt", 1).is_ok());
        assert!(check_file("a.txt", 20 * 1024 * 1024).is_ok());
        assert!(check_file("a.txt", 26 * 1024 * 1024).is_err());
        assert!(check_file("a.txt", MAX_BYTES).is_ok());
        let e = check_file("big.bin", MAX_BYTES + 1).unwrap_err();
        assert!(e.contains("25"), "{e}");
        assert!(check_file("empty.txt", 0).is_err());
        assert!(check_file("", 5).is_err());
    }

    #[test]
    fn uploaded_needs_id_and_name() {
        let u = parse_uploaded(
            &json!({"id": 4, "name": "<b>a</b>.txt", "size": 12, "mime": "text/plain"}),
        )
        .unwrap();
        assert_eq!((u.id, u.name.as_str(), u.size), (4, "<b>a</b>.txt", 12));
        assert!(parse_uploaded(&json!({"name": "x"})).is_none());
        assert!(parse_uploaded(&json!({"id": 1})).is_none());
    }

    #[test]
    fn ids_skip_non_numeric() {
        assert_eq!(
            attachment_ids(&["4".into(), "x".into(), "9".into()]),
            vec![4, 9]
        );
    }
}
