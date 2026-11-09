//! `bar.*` frames as the controllers see them (docs/reference/api.md, section 4.4). Unknown fields are ignored; missing fields and `null` mean "no data".

use serde::Deserialize;
use serde_json::Value;

/// `bar.sys`: percentages 0–100 and temperature in °C.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct SysFrame {
    pub cpu: Option<f64>,
    pub gpu: Option<f64>,
    pub ram: Option<f64>,
    pub disk: Option<f64>,
    pub temp: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum QuotaKind {
    #[default]
    #[serde(rename = "api_key")]
    ApiKey,
    #[serde(rename = "local")]
    Local,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Limit {
    pub limit: u64,
    pub remaining: u64,
    #[serde(default)]
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub requests_per_min: Option<Limit>,
    pub tokens_per_min: Option<Limit>,
}

/// A single quota from `bar.quota`. `slug`, `label`, `provider` are fields on top of `Quota` from the OpenAPI draft.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct QuotaItem {
    pub account_id: i64,
    pub slug: String,
    pub label: String,
    pub provider: String,
    pub kind: QuotaKind,
    pub window_h: Option<f64>,
    pub used: Option<f64>,
    pub rate: Option<f64>,
    pub reset_at: Option<i64>,
    pub load: Option<f64>,
    pub usage: Option<Usage>,
    pub limits: Option<Limits>,
}

/// `bar.weather`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct WeatherFrame {
    pub temp_c: Option<f64>,
    pub icon: Option<String>,
    pub text: Option<String>,
    pub place: Option<String>,
    pub updated_at: Option<i64>,
}

/// Parses `bar.sys`; a non-object yields a frame without data.
pub fn parse_sys(data: &Value) -> SysFrame {
    serde_json::from_value(data.clone()).unwrap_or_default()
}

/// Parses `bar.weather`; a non-object yields a frame without data.
pub fn parse_weather(data: &Value) -> WeatherFrame {
    serde_json::from_value(data.clone()).unwrap_or_default()
}

/// Snapshot of `bar.quota`: `None` means no data (`items` is `null`-like empty or the frame failed to parse).
pub fn parse_quota_snapshot(data: &Value) -> Option<Vec<QuotaItem>> {
    let items = data.get("items")?.as_array()?;
    items
        .iter()
        .map(|i| serde_json::from_value(i.clone()).ok())
        .collect()
}

/// Delta of `bar.quota`: a single quota.
pub fn parse_quota_item(data: &Value) -> Option<QuotaItem> {
    serde_json::from_value(data.clone()).ok()
}
