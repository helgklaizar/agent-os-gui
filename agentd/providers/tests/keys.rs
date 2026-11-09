//! Secret store `keys.env` (unit, `secret_*`).

use std::os::unix::fs::PermissionsExt;

use agentd_core::{Secret, SecretError, SecretStore};
use agentd_providers::KeysFile;

const VALUE: &str = "sk-test-VALUE-01234";

fn store() -> (tempfile::TempDir, KeysFile) {
    let dir = tempfile::tempdir().unwrap();
    let keys = KeysFile::new(dir.path().join(".agentos/keys.env"));
    (dir, keys)
}

fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn secret_put_get_roundtrip_with_hard_values() {
    let (_d, k) = store();
    let hard = [
        VALUE,
        "quotes \" and \\ and = and # and space ",
        "line1\nline2\r\n",
        "$HOME `x` 'y'",
        "unicode 🔑",
    ];
    for (i, v) in hard.iter().enumerate() {
        k.put(
            &format!("account.{}", i + 1),
            Secret::new((*v).to_string()),
            10,
        )
        .await
        .unwrap();
    }
    for (i, v) in hard.iter().enumerate() {
        let got = k.get(&format!("account.{}", i + 1)).await.unwrap();
        assert_eq!(got.expose(), v);
    }
    assert!(k.get("account.99").await.is_none());
}

#[tokio::test]
async fn secret_put_write_only() {
    let (_d, k) = store();
    k.put("provider.anthropic", Secret::new(VALUE.into()), 1234)
        .await
        .unwrap();
    let list = k.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].secret_ref, "provider.anthropic");
    assert!(list[0].set);
    assert_eq!(list[0].updated_at, 1234);
    let json = serde_json::to_string(&list).unwrap();
    assert!(!json.contains(VALUE), "{json}");
    assert_eq!(
        json,
        r#"[{"ref":"provider.anthropic","set":true,"updated_at":1234}]"#
    );
    assert!(!format!("{k:?}").contains(VALUE));
}

#[tokio::test]
async fn secret_file_mode_0600_atomic() {
    let (_d, k) = store();
    k.put("provider.a", Secret::new("v1".into()), 1)
        .await
        .unwrap();
    assert_eq!(mode(k.path()), 0o600);
    assert_eq!(mode(k.path().parent().unwrap()), 0o700);
    // Overwrite keeps the old content if the write failed: the directory is read-only.
    let dir = k.path().parent().unwrap().to_path_buf();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = k.put("provider.a", Secret::new("v2".into()), 2).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(r.is_err(), "writing to a read-only directory must fail");
    assert_eq!(k.get("provider.a").await.unwrap().expose(), "v1");
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["keys.env"], "no intermediate files: {names:?}");
}

#[tokio::test]
async fn secret_rejects_bad_ref_and_value_without_touching_file() {
    let (_d, k) = store();
    for r in [
        "",
        "../x",
        "provider.A",
        "keys.env",
        "account.0",
        "provider.a=b",
    ] {
        let e = k.put(r, Secret::new("v".into()), 1).await.unwrap_err();
        assert!(matches!(e, SecretError::Invalid(_)), "{r}: {e:?}");
    }
    for v in ["", "a\0b"] {
        assert!(matches!(
            k.put("provider.a", Secret::new(v.into()), 1).await,
            Err(SecretError::Invalid(_))
        ));
    }
    let big = "x".repeat(agentd_core::MAX_SECRET_BYTES + 1);
    assert!(matches!(
        k.put("provider.a", Secret::new(big), 1).await,
        Err(SecretError::Invalid(_))
    ));
    assert!(!k.path().exists(), "file not created");
}

#[tokio::test]
async fn secret_delete_and_erase() {
    let (_d, k) = store();
    k.put("account.3", Secret::new("a".into()), 1)
        .await
        .unwrap();
    k.put("account.4", Secret::new("b".into()), 1)
        .await
        .unwrap();
    assert!(k.delete("account.3").await.unwrap());
    assert!(
        !k.delete("account.3").await.unwrap(),
        "repeated removal: nothing was there"
    );
    assert!(k.get("account.3").await.is_none());
    assert_eq!(k.get("account.4").await.unwrap().expose(), "b");
    let raw = std::fs::read_to_string(k.path()).unwrap();
    assert!(!raw.contains("account.3"));
    agentd_core::SecretEraser::erase(&k, "account.4").await;
    assert!(k.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn secret_foreign_lines_survive_rewrite() {
    let (_d, k) = store();
    std::fs::create_dir_all(k.path().parent().unwrap()).unwrap();
    std::fs::write(k.path(), "# my note\nEXTRA=1\n").unwrap();
    k.put("provider.a", Secret::new("v".into()), 1)
        .await
        .unwrap();
    let raw = std::fs::read_to_string(k.path()).unwrap();
    assert!(
        raw.contains("EXTRA=1") && raw.contains("# my note"),
        "{raw}"
    );
    assert_eq!(mode(k.path()), 0o600);
}
