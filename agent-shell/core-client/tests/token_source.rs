//! Reading the token: file (tests only), value, rejection of bad content. The fd is checked in integration-tests (`gui_token`).
#![cfg(feature = "test-token-file")]

use std::path::PathBuf;

use core_client::token::TokenError;
use core_client::{Token, TokenSource};

fn file_with(dir: &tempfile::TempDir, name: &str, content: &[u8]) -> PathBuf {
    let p = dir.path().join(name);
    assert!(std::fs::write(&p, content).is_ok());
    p
}

fn read(path: PathBuf) -> Result<Token, TokenError> {
    TokenSource::TestFile(path).read()
}

#[test]
fn test_file_is_trimmed_and_value_passes_through() {
    let dir = tempfile::tempdir().map_err(|e| e.to_string());
    let Ok(dir) = dir else { panic!("{dir:?}") };
    let t = read(file_with(&dir, "ok", b"agt_abc-DEF_123\n"));
    assert!(
        matches!(&t, Ok(t) if t == &Token::new("agt_abc-DEF_123")),
        "{t:?}"
    );
    let v = TokenSource::Value(Token::new("agt_x")).read();
    assert!(matches!(&v, Ok(t) if t == &Token::new("agt_x")));
}

#[test]
fn bad_token_files_are_rejected() {
    let dir = tempfile::tempdir().map_err(|e| e.to_string());
    let Ok(dir) = dir else { panic!("{dir:?}") };
    let long = vec![b'a'; 513];
    for (name, content) in [
        ("empty", &b""[..]),
        ("blank", b" \n\t"),
        ("inner space", b"agt_a b"),
        ("newline inside", b"agt_a\nagt_b"),
        ("control", b"agt_\x01x"),
        ("non-utf8", &[0xff, 0xfe, b'a']),
        ("non-ascii", "agt_tökén".as_bytes()),
        ("too long", &long),
    ] {
        let r = read(file_with(&dir, name, content));
        assert!(matches!(r, Err(TokenError::Invalid)), "{name}: {r:?}");
    }
    assert!(
        matches!(read(dir.path().join("missing")), Err(TokenError::Read(_))),
        "no file: read error without the path in the text"
    );
    let msg = read(dir.path().join("missing"))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        !msg.contains(dir.path().to_string_lossy().as_ref()),
        "the path does not show up in the error text"
    );
}

#[test]
fn test_env_value_selects_file_source() {
    assert!(
        matches!(TokenSource::from_test_env_value(Some("/x/t")), Some(TokenSource::TestFile(p)) if p.as_os_str() == "/x/t")
    );
    assert!(TokenSource::from_test_env_value(Some("")).is_none());
    assert!(TokenSource::from_test_env_value(None).is_none());
}
