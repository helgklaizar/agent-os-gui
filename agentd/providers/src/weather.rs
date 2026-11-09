//! Open-Meteo weather without a key: `GET /v1/forecast?latitude=..&longitude=..&current=temperature_2m,weather_code`.
//! The request carries only coordinates; the response is parsed strictly, unavailability gives `None` (no defaults).

use agentd_core::WeatherNow;
use serde_json::Value;

use crate::HttpClient;

pub const DEFAULT_BASE: &str = "https://api.open-meteo.com";

/// Description by WMO code (short, English).
#[must_use]
pub fn describe(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1..=3 => "cloudy",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 | 80..=82 => "rain",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "thunderstorm",
        _ => "no description",
    }
}

/// Response parsing: temperature and weather code are required and plausible.
#[must_use]
pub fn parse(body: &[u8]) -> Option<WeatherNow> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let cur = &v["current"];
    let temp = cur["temperature_2m"]
        .as_f64()
        .filter(|t| (-90.0..=70.0).contains(t))?;
    let code = cur["weather_code"]
        .as_i64()
        .filter(|c| (0..=99).contains(c))?;
    Some(WeatherNow {
        temp,
        code,
        desc: describe(code).to_string(),
    })
}

pub async fn fetch(http: &HttpClient, base: &str, lat: f64, lon: f64) -> Option<WeatherNow> {
    let url = format!(
        "{}/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code",
        base.trim_end_matches('/')
    );
    match http.call_json("GET", &url, &[], Vec::new()).await {
        Ok((200, body)) => parse(&body),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plausible_values_only() {
        let ok = parse(br#"{"current":{"temperature_2m":21.5,"weather_code":3,"x":1}}"#).unwrap();
        assert_eq!((ok.temp, ok.code, ok.desc.as_str()), (21.5, 3, "cloudy"));
        for bad in [
            &br#"{}"#[..],
            br#"{"current":{"temperature_2m":999,"weather_code":3}}"#,
            br#"{"current":{"temperature_2m":1}}"#,
            b"not json",
        ] {
            assert!(parse(bad).is_none());
        }
    }
}
