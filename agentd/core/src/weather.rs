//! Weather for `bar.weather`: a `{temp, code, desc, ts}` frame; no data — all fields `null`, no dummy values.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// Current weather from the source.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherNow {
    pub temp: f64,
    pub code: i64,
    pub desc: String,
}

/// Last frame for the topic `snapshot`.
#[derive(Clone)]
pub struct WeatherSnapshot(Arc<Mutex<Value>>);

impl Default for WeatherSnapshot {
    fn default() -> WeatherSnapshot {
        WeatherSnapshot(Arc::new(Mutex::new(Self::frame(None, 0))))
    }
}

impl WeatherSnapshot {
    #[must_use]
    pub fn frame(now: Option<&WeatherNow>, ts: i64) -> Value {
        match now {
            Some(w) => json!({"temp": w.temp, "code": w.code, "desc": w.desc, "ts": ts}),
            None => json!({"temp": null, "code": null, "desc": null, "ts": ts}),
        }
    }

    pub fn set(&self, v: Value) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = v;
    }

    #[must_use]
    pub fn get(&self) -> Value {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Coordinates from the `account` page (`lat`, `lon`); out of range or not numbers — `None`.
#[must_use]
pub fn location_of(account: &Value) -> Option<(f64, f64)> {
    let lat = account["lat"]
        .as_f64()
        .filter(|v| (-90.0..=90.0).contains(v))?;
    let lon = account["lon"]
        .as_f64()
        .filter(|v| (-180.0..=180.0).contains(v))?;
    Some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_needs_both_in_range() {
        assert_eq!(
            location_of(&json!({"lat": 32.8, "lon": 35})),
            Some((32.8, 35.0))
        );
        for bad in [
            json!({}),
            json!({"lat": 91, "lon": 0}),
            json!({"lat": "1", "lon": 0}),
            json!({"lat": 1}),
        ] {
            assert!(location_of(&bad).is_none());
        }
    }

    #[test]
    fn empty_frame_is_all_null() {
        let f = WeatherSnapshot::frame(None, 5);
        assert!(f["temp"].is_null() && f["code"].is_null() && f["desc"].is_null());
    }
}
