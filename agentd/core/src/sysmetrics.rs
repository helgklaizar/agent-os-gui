//! Latest hardware metrics sample for the `bar.sys` topic `snapshot`: written by the collector, read by the API.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

#[derive(Clone)]
pub struct SysSnapshot(Arc<Mutex<Value>>);

impl Default for SysSnapshot {
    fn default() -> SysSnapshot {
        SysSnapshot(Arc::new(Mutex::new(Self::frame(
            None, None, None, None, None,
        ))))
    }
}

impl SysSnapshot {
    /// `bar.sys` frame: percentages 0–100, temperature °C, no sensor — `null`.
    #[must_use]
    pub fn frame(
        cpu: Option<f64>,
        gpu: Option<f64>,
        ram: Option<f64>,
        disk: Option<f64>,
        temp: Option<f64>,
    ) -> Value {
        json!({"cpu": cpu, "gpu": gpu, "ram": ram, "disk": disk, "temp": temp})
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
