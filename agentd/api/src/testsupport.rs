//! Test build only (A1-11): `POST /v1/_test/advance {"seconds": n}` moves the controlled clock.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use serde_json::{Value, json};

use agentd_core::FieldError;

use crate::body::read_json;
use crate::error::ApiError;
use crate::state::ApiState;

pub(crate) async fn advance(
    State(state): State<Arc<ApiState>>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let v = read_json(body).await?;
    let secs = v.get("seconds").and_then(Value::as_i64).filter(|s| *s > 0);
    let Some(secs) = secs else {
        return Err(ApiError::validation_failed(&[FieldError::new(
            "seconds",
            "a positive integer is required",
        )]));
    };
    if !state.clock.advance_secs(secs) {
        return Err(ApiError::conflict("clock_not_manual"));
    }
    Ok(Json(json!({ "now": state.clock.now_unix() })))
}

/// `POST /v1/_test/observer {"available": bool}`: simulates an unavailable observer.
pub(crate) async fn observer(
    State(state): State<Arc<ApiState>>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let v = read_json(body).await?;
    let Some(on) = v.get("available").and_then(Value::as_bool) else {
        return Err(ApiError::validation_failed(&[FieldError::new(
            "available",
            "true or false is required",
        )]));
    };
    let obs = state
        .services()
        .and_then(|s| s.observer.clone())
        .ok_or_else(|| ApiError::core_down("Observer not connected"))?;
    if !obs.force_available(on) {
        return Err(ApiError::conflict("observer_not_switchable"));
    }
    Ok(Json(json!({ "available": on })))
}

/// `POST /v1/_test/indicator {"id", "state", "value"?, "text"?}`: an indicator frame through the engine's production entry.
pub(crate) async fn indicator(
    State(state): State<Arc<ApiState>>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    use agentd_core::{IndicatorError, IndicatorState, Reading};
    let v = read_json(body).await?;
    let mut errs = Vec::new();
    let id = v.get("id").and_then(Value::as_str).unwrap_or("");
    let st = v
        .get("state")
        .and_then(Value::as_str)
        .and_then(IndicatorState::parse);
    if id.is_empty() {
        errs.push(FieldError::new("id", "a string is required"));
    }
    if st.is_none() {
        errs.push(FieldError::new("state", "ok, warn, bad, nodata or unknown"));
    }
    let (true, Some(st)) = (errs.is_empty(), st) else {
        return Err(ApiError::validation_failed(&errs));
    };
    let engine = state
        .services()
        .and_then(|s| s.indicators.clone())
        .ok_or_else(|| ApiError::core_down("Indicators unavailable"))?;
    let reading = Reading {
        state: st,
        value: v.get("value").filter(|x| !x.is_null()).cloned(),
        text: v.get("text").and_then(Value::as_str).map(str::to_string),
    };
    match engine.push(id, reading).await {
        Ok(o) => Ok(Json(
            json!({"transitioned": o.transitioned, "frame": o.frame}),
        )),
        Err(IndicatorError::Invalid(f)) => Err(ApiError::validation_failed(&f)),
        Err(IndicatorError::Unavailable) => Err(ApiError::core_down("Indicators unavailable")),
    }
}

/// `POST /v1/_test/ingest {account_id, ext_id, handle, body, direction?}`: an inbound message through the gateway's production entry.
pub(crate) async fn ingest(
    State(state): State<Arc<ApiState>>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    use agentd_core::{Direction, GatewayError, Ingest};
    let v = read_json(body).await?;
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let direction = match v.get("direction").and_then(Value::as_str) {
        None | Some("in") => Direction::In,
        Some("out") => Direction::Out,
        Some(_) => {
            return Err(ApiError::validation_failed(&[FieldError::new(
                "direction",
                "in or out",
            )]));
        }
    };
    let gw = state
        .services()
        .and_then(|s| s.gateway.clone())
        .ok_or_else(|| ApiError::core_down("Gateway unavailable"))?;
    let input = Ingest {
        account_id: v.get("account_id").and_then(Value::as_i64).unwrap_or(0),
        ext_id: text("ext_id"),
        direction,
        handle: text("handle"),
        body: text("body"),
        ts: state.clock.now_unix(),
    };
    match gw.ingest(input).await {
        Ok(done) => Ok(Json(done.to_json())),
        Err(GatewayError::Invalid(f)) => Err(ApiError::validation_failed(&f)),
        Err(GatewayError::NoAccount) => Err(ApiError::new(404, "not_found", "No such account")),
        Err(GatewayError::Unavailable) => Err(ApiError::core_down("Gateway unavailable")),
    }
}

/// `POST /v1/_test/channel {...}`: test channel control; `refresh: <id>` and `poll: true` call the production methods.
pub(crate) async fn channel(
    State(state): State<Arc<ApiState>>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let v = read_json(body).await?;
    let services = state
        .services()
        .ok_or_else(|| ApiError::core_down("Services not connected"))?;
    let control = services
        .channel_control
        .clone()
        .ok_or_else(|| ApiError::conflict("no_test_channel"))?;
    let out = control.apply(v.clone()).await;
    if let (Some(id), Some(ch)) = (v.get("refresh").and_then(Value::as_i64), &services.channels) {
        ch.refresh(id).await;
    }
    if v.get("poll") == Some(&Value::Bool(true))
        && let Some(ch) = &services.channels
    {
        ch.poll_all().await;
    }
    Ok(Json(out))
}
