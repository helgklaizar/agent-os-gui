//! Theme exchange with the core against a real `fake-agentd` over a unix socket.

use std::cell::Cell;

use controllers::Theme;
use core_client::api::{self, ErrorCode};
use core_client::blocking::BlockingClient;
use core_client::{ClientError, Endpoint, Token};
use fake_agentd::{Config, FakeAgentd, Kind, Scope, TokenSpec};
use ui::theme_sync::{
    MAX_ATTEMPTS, SyncError, ThemeApi, from_api, patch_theme, read_theme, to_api,
};

async fn core() -> FakeAgentd {
    FakeAgentd::start(Config::temp().no_ticker())
        .await
        .expect("fake-agentd did not start")
}

fn client(d: &FakeAgentd, token: &str) -> BlockingClient {
    BlockingClient::new(
        Endpoint::Unix(d.socket_path().to_path_buf()),
        Token::new(token),
    )
    .expect("client")
}

fn admin(d: &FakeAgentd) -> BlockingClient {
    client(d, d.admin_token())
}

/// A foreign client wedges in between our read of `rev` and the write: the core answers with a real `412`.
struct Racing<'a> {
    inner: &'a BlockingClient,
    other: &'a BlockingClient,
    times: u32,
    raced: Cell<u32>,
}

impl ThemeApi for Racing<'_> {
    fn get(&self) -> Result<api::ThemePage, ClientError> {
        let page = self.inner.theme()?;
        if self.raced.get() < self.times {
            self.raced.set(self.raced.get() + 1);
            self.other.set_theme(page.rev, api::Theme::Light)?;
        }
        Ok(page)
    }

    fn patch(&self, rev: i64, theme: api::Theme) -> Result<api::ThemePage, ClientError> {
        self.inner.set_theme(rev, theme)
    }
}

fn patches(d: &FakeAgentd) -> Vec<u16> {
    d.requests()
        .iter()
        .filter(|r| r.method == "PATCH" && r.path == "/v1/settings/theme")
        .map(|r| r.status)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_patch_sends_if_match_and_updates_core() {
    let d = core().await;
    let c = admin(&d);
    let (page, after) = tokio::task::spawn_blocking(move || {
        let page = patch_theme(&c, Theme::Dark);
        (page, c.theme())
    })
    .await
    .expect("thread");
    let page = page.expect("theme write");
    assert_eq!((page.theme, page.rev), (api::Theme::Dark, 2), "rev grows");
    assert!(matches!(after, Ok(p) if p.theme == api::Theme::Dark && p.rev == 2));
    assert_eq!(patches(&d), vec![200], "one write, no retries");
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_patch_retries_after_412() {
    let d = core().await;
    let (mine, other) = (admin(&d), admin(&d));
    let result = tokio::task::spawn_blocking(move || {
        let racing = Racing {
            inner: &mine,
            other: &other,
            times: 1,
            raced: Cell::new(0),
        };
        let page = patch_theme(&racing, Theme::Dark);
        (page, mine.theme())
    })
    .await
    .expect("thread");
    let page = result.0.expect("after 412 the write must succeed");
    assert_eq!((page.theme, page.rev), (api::Theme::Dark, 3));
    assert!(matches!(result.1, Ok(p) if p.theme == api::Theme::Dark));
    // the foreign write (200), ours with the old rev (412), ours with the new one (200)
    assert_eq!(patches(&d), vec![200, 412, 200]);
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_patch_gives_up_after_repeated_412() {
    let d = core().await;
    let (mine, other) = (admin(&d), admin(&d));
    let outcome = tokio::task::spawn_blocking(move || {
        let racing = Racing {
            inner: &mine,
            other: &other,
            times: u32::MAX,
            raced: Cell::new(0),
        };
        patch_theme(&racing, Theme::Dark)
    })
    .await
    .expect("thread");
    assert!(matches!(outcome, Err(SyncError::Conflict)), "{outcome:?}");
    let ours_412 = patches(&d).iter().filter(|s| **s == 412).count();
    assert_eq!(ours_412 as u32, MAX_ATTEMPTS, "retries are bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_patch_forbidden_is_not_retried() {
    let d = core().await;
    let read = d
        .issue(TokenSpec::new(Scope::Read, Kind::Test))
        .expect("read token")
        .token;
    let c = client(&d, &read);
    let outcome = tokio::task::spawn_blocking(move || patch_theme(&c, Theme::Dark))
        .await
        .expect("thread");
    match outcome {
        Err(SyncError::Client(e)) => assert!(
            matches!(e.api(), Some(a) if a.status == 403 && a.code == ErrorCode::InsufficientScope),
            "{e:?}"
        ),
        other => panic!("expected 403 from the core, got {other:?}"),
    }
    assert_eq!(patches(&d), vec![403], "a refusal is not retried");
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_patch_offline_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().expect("directory");
    let dead = BlockingClient::new(
        Endpoint::Unix(dir.path().join("no-such.sock")),
        Token::new("<token>"),
    )
    .expect("client");
    let outcome =
        tokio::task::spawn_blocking(move || (patch_theme(&dead, Theme::Dark), read_theme(&dead)))
            .await
            .expect("thread");
    assert!(
        matches!(outcome.0, Err(SyncError::Client(ClientError::Transport(_)))),
        "{:?}",
        outcome.0
    );
    assert!(
        matches!(outcome.1, Err(SyncError::Client(_))),
        "{:?}",
        outcome.1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn theme_read_returns_core_value() {
    let d = core().await;
    let (a, b) = (admin(&d), admin(&d));
    let (first, second) = tokio::task::spawn_blocking(move || {
        let first = read_theme(&a);
        a.set_theme(1, api::Theme::Light).expect("write");
        (first, read_theme(&b))
    })
    .await
    .expect("thread");
    assert!(matches!(first, Ok(Theme::Auto)), "{first:?}");
    assert!(matches!(second, Ok(Theme::Light)), "{second:?}");
}

#[test]
fn theme_conversion_roundtrip() {
    for t in [Theme::Auto, Theme::Dark, Theme::Light] {
        assert_eq!(from_api(to_api(t)), t);
    }
    assert_eq!(to_api(Theme::Dark), api::Theme::Dark);
    assert_eq!(to_api(Theme::Light), api::Theme::Light);
    assert_eq!(to_api(Theme::Auto), api::Theme::Auto);
}
