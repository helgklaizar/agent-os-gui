//! Browser odds and ends: time, UUID v7, timezone offset.

/// Current time, unix seconds.
pub fn unix_now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Local timezone offset from UTC, seconds (for time labels in the chat).
pub fn utc_offset_secs() -> i32 {
    // `getTimezoneOffset` is minutes "UTC minus local", the sign is opposite.
    -((js_sys::Date::new_0().get_timezone_offset()) as i32) * 60
}

fn rand_bytes(n: usize) -> Vec<u8> {
    (0..n)
        .map(|_| (js_sys::Math::random() * 256.0) as u8)
        .collect()
}

/// UUID v7: 48 bits of milliseconds, version 7, RFC 4122 variant, the rest random (`Math.random` is enough:
/// the value is a request id, not a secret, see docs/reference/api.md section 1.3).
pub fn uuid7() -> String {
    let ms = js_sys::Date::now() as u64;
    let mut b = [0u8; 16];
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    b[6..].copy_from_slice(&rand_bytes(10));
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Tab storage (`sessionStorage`); any failure gives `None` or is silent.
fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok()?
}

pub fn stored_token() -> Option<String> {
    session_storage()?.get_item("agentd.token").ok()?
}

pub fn store_token(token: &str) {
    if let Some(s) = session_storage() {
        drop(s.set_item("agentd.token", token));
    }
}

pub fn forget_token() {
    if let Some(s) = session_storage() {
        drop(s.remove_item("agentd.token"));
    }
}

/// Local time for labels: `dd.mm hh:mm`.
pub fn fmt_ts(ts: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    format!(
        "{:02}.{:02} {:02}:{:02}",
        d.get_date(),
        d.get_month() + 1,
        d.get_hours(),
        d.get_minutes()
    )
}

const OUTBOX_KEY: &str = "agentd.outbox";

/// Outbox queue from tab storage; any storage failure or corrupt entry gives an empty queue.
pub fn load_outbox() -> Vec<crate::outbox::Outgoing> {
    session_storage()
        .and_then(|s| s.get_item(OUTBOX_KEY).ok().flatten())
        .map(|t| crate::outbox::decode(&t))
        .unwrap_or_default()
}

/// Saves the queue; an empty queue removes the entry. Storage errors (private mode, quota) are silently skipped.
pub fn store_outbox(encoded: &str) {
    if let Some(s) = session_storage() {
        if encoded == "[]" {
            drop(s.remove_item(OUTBOX_KEY));
        } else {
            drop(s.set_item(OUTBOX_KEY, encoded));
        }
    }
}
