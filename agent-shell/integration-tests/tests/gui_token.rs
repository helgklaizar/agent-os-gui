//! The GUI token arrives through an fd and is never stored as a file.

mod common;

use std::time::Duration;

use common::quiet;
use fake_agentd::{Kind, Scope, TokenSpec};

#[tokio::test]
async fn gui_token_not_in_file() {
    let started = std::time::SystemTime::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(std::time::UNIX_EPOCH);
    let d = quiet().await;
    let gui = d.issue(TokenSpec::new(Scope::Admin, Kind::Gui));
    let Ok(gui) = gui else { panic!("{gui:?}") };
    let (reader, mut writer) = match std::io::pipe() {
        Ok(p) => p,
        Err(e) => panic!("pipe: {e}"),
    };
    std::io::Write::write_all(&mut writer, gui.token.as_bytes())
        .unwrap_or_else(|e| panic!("pipe write: {e}"));
    drop(writer);
    let fd = std::os::fd::AsRawFd::as_raw_fd(&reader);
    let client = match core_client::CoreClient::start(common::core_config_with(
        &d,
        core_client::TokenSource::Fd(fd),
    )) {
        Ok(c) => c,
        Err(e) => panic!("client with an fd did not start: {e}"),
    };
    let rx = client.events();
    let up = common::wait_state(&rx, core_client::ConnState::Connected).await;
    assert!(
        up.is_ok(),
        "client with a token from an fd connected: {up:?}"
    );
    // the token is neither in the mock's directory nor in common places where the client could have written it
    let needle = gui.token.as_bytes();
    let mut roots = vec![d.dir().to_path_buf(), std::env::temp_dir()];
    roots.extend(std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from));
    let mut hits = Vec::new();
    for root in roots {
        find_token(&root, needle, started, 0, &mut hits);
    }
    assert!(hits.is_empty(), "token found in files: {hits:?}");
    assert!(
        std::env::vars().all(|(_, v)| !v.contains(&gui.token)),
        "the token is not in the environment"
    );
    #[cfg(target_os = "linux")]
    {
        let environ = std::fs::read("/proc/self/environ").unwrap_or_default();
        assert!(
            !environ
                .windows(gui.token.len())
                .any(|w| w == gui.token.as_bytes()),
            "/proc/self/environ without the token"
        );
    }
    client.shutdown();
}

/// Looks for `needle` in files modified after `since`: depth up to 4, files up to 4 MiB, symlinks are not followed.
fn find_token(
    dir: &std::path::Path,
    needle: &[u8],
    since: std::time::SystemTime,
    depth: u32,
    hits: &mut Vec<std::path::PathBuf>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if depth < 4 {
                find_token(&path, needle, since, depth + 1, hits);
            }
        } else if meta.is_file()
            && meta.len() <= 4 << 20
            && meta.modified().is_ok_and(|m| m >= since)
            && std::fs::read(&path).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle))
        {
            hits.push(path);
        }
    }
}
