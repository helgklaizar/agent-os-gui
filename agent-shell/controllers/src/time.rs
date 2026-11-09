//! `Instant` for controllers: `std::time::Instant::now()` is unavailable on `wasm32`, so the type comes from `web-time`.
//! On other platforms it is the same `std::time::Instant`.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
