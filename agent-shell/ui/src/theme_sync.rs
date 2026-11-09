//! Theme exchange with the core: read and write via `PATCH` with `If-Match`, on `412` re-read and retry.

use std::fmt;

use core_client::ClientError;
use core_client::api::{self, ErrorCode, ThemePage};
use core_client::blocking::BlockingClient;

/// How many times to try the write on `412` before giving up.
pub const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug)]
pub enum SyncError {
    Client(ClientError),
    /// `412` on each of the [`MAX_ATTEMPTS`] attempts.
    Conflict,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Client(e) => write!(f, "{e}"),
            SyncError::Conflict => write!(f, "theme changed concurrently {MAX_ATTEMPTS} times"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<ClientError> for SyncError {
    fn from(e: ClientError) -> Self {
        SyncError::Client(e)
    }
}

/// Source of the core theme. In the product this is [`BlockingClient`]; a test substitutes a wrapper that injects a foreign write.
pub trait ThemeApi {
    fn get(&self) -> Result<ThemePage, ClientError>;
    fn patch(&self, rev: i64, theme: api::Theme) -> Result<ThemePage, ClientError>;
}

impl ThemeApi for BlockingClient {
    fn get(&self) -> Result<ThemePage, ClientError> {
        self.theme()
    }

    fn patch(&self, rev: i64, theme: api::Theme) -> Result<ThemePage, ClientError> {
        self.set_theme(rev, theme)
    }
}

pub fn to_api(theme: controllers::Theme) -> api::Theme {
    match theme {
        controllers::Theme::Auto => api::Theme::Auto,
        controllers::Theme::Dark => api::Theme::Dark,
        controllers::Theme::Light => api::Theme::Light,
    }
}

pub fn from_api(theme: api::Theme) -> controllers::Theme {
    match theme {
        api::Theme::Auto => controllers::Theme::Auto,
        api::Theme::Dark => controllers::Theme::Dark,
        api::Theme::Light => controllers::Theme::Light,
    }
}

/// Theme value in the core.
pub fn read_theme(api: &dyn ThemeApi) -> Result<controllers::Theme, SyncError> {
    Ok(from_api(api.get()?.theme))
}

/// Write the theme to the core: read `rev`, `PATCH` with `If-Match`, on `412` retry with the new `rev`.
/// Any other error (`403`, connection loss) is returned at once, without retries.
pub fn patch_theme(api: &dyn ThemeApi, theme: controllers::Theme) -> Result<ThemePage, SyncError> {
    for _ in 0..MAX_ATTEMPTS {
        let current = api.get()?;
        match api.patch(current.rev, to_api(theme)) {
            Ok(page) => return Ok(page),
            Err(e) if is_precondition_failed(&e) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(SyncError::Conflict)
}

fn is_precondition_failed(e: &ClientError) -> bool {
    e.api()
        .is_some_and(|a| a.status == 412 && a.code == ErrorCode::PreconditionFailed)
}
