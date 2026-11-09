//! Boundary: fact text up to 4096 bytes inclusive is accepted, one byte more — `invalid_args`, the fact is not written.

use std::sync::{Arc, Mutex};

use agentd_core::{
    Action, ActionCall, ActionExecutor, MemoryError, MemoryExecutor, MemoryStore, PortFuture,
    SystemClock,
};
use serde_json::json;

#[derive(Default)]
struct Rec(Mutex<Vec<String>>);

impl MemoryStore for Rec {
    fn add(
        &self,
        _session_id: i64,
        text: String,
        _now: i64,
    ) -> PortFuture<'_, Result<i64, MemoryError>> {
        Box::pin(async move {
            let mut v = self.0.lock().unwrap();
            v.push(text);
            Ok(i64::try_from(v.len()).unwrap())
        })
    }
}

async fn write(text: &str) -> (Result<serde_json::Value, &'static str>, usize) {
    let store = Arc::new(Rec::default());
    let ex = MemoryExecutor::new(store.clone(), Arc::new(SystemClock));
    let call = ActionCall {
        action: Action::MemoryWrite,
        target: String::new(),
        args: json!({"text": text}),
        session_id: 1,
    };
    let r = ex.execute(&call).await.map_err(|e| e.code);
    let n = store.0.lock().unwrap().len();
    (r, n)
}

#[tokio::test]
async fn memory_fact_size_limit_is_exactly_4096_bytes() {
    let (ok, n) = write(&"a".repeat(4096)).await;
    assert!(ok.is_ok(), "4096 bytes accepted: {ok:?}");
    assert_eq!(n, 1);
    let (bad, n) = write(&"a".repeat(4097)).await;
    assert_eq!(bad.unwrap_err(), "invalid_args", "4097 bytes rejected");
    assert_eq!(n, 0, "a rejected fact does not reach the store");
}

#[tokio::test]
async fn memory_fact_size_counts_bytes_not_chars() {
    // 2048 two-byte characters = 4096 bytes — accepted; one more — not.
    let (ok, _) = write(&"é".repeat(2048)).await;
    assert!(ok.is_ok(), "{ok:?}");
    let (bad, _) = write(&"é".repeat(2049)).await;
    assert_eq!(bad.unwrap_err(), "invalid_args");
}
