//! Invalid token limit (429, `Retry-After`) and the list of known contract drifts against `openapi-draft.yaml`.

use std::time::Duration;

use crate::openapi::Spec;
use crate::rawhttp::{Req, send};
use crate::yaml;
use crate::{KNOWN_CODE_DRIFT, KNOWN_STATUS_DRIFT, assert_clean, mock};

#[tokio::test(flavor = "multi_thread")]
async fn contract_rate_limit_429_body_and_retry_after() {
    let d = mock().await;
    let spec = Spec::load();
    let mut last = None;
    for i in 0..6 {
        let bad = format!("agt_{i:0>43}");
        last = Some(
            send(
                d.socket_path(),
                Req::new("GET", "/v1/settings/theme").token(&bad),
            )
            .await,
        );
    }
    let Some(r) = last else { panic!("no response") };
    assert_eq!(r.status, 429, "the sixth invalid attempt in a minute");
    assert!(
        r.header("retry-after")
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|s| (1..=60).contains(&s)),
        "Retry-After in seconds"
    );
    assert_clean(
        "429",
        spec.validate(spec.schema("Error"), &r.json(), "Error"),
    );
    assert_eq!(r.json()["error"]["code"].as_str(), Some("rate_limited"));
    assert_eq!(r.json()["error"]["retryable"].as_bool(), Some(true));
    tokio::time::sleep(Duration::from_millis(1)).await;
}

#[test]
fn contract_known_drift_is_still_drift() {
    let spec = Spec::load();
    let enum_codes: Vec<&str> = spec
        .schema("Error")
        .path(&["properties", "error", "properties", "code", "enum"])
        .map(yaml::Y::strs)
        .unwrap_or_default();
    for (code, why) in KNOWN_CODE_DRIFT {
        assert!(
            !enum_codes.contains(code),
            "code {code} is already in the Error enum ({why}): remove it from KNOWN_CODE_DRIFT"
        );
        assert!(!why.is_empty(), "the entry has no reason");
    }
    for (m, p, s) in KNOWN_STATUS_DRIFT {
        assert!(
            !spec.statuses(m, p).contains(s),
            "{m} {p} already describes {s}: remove the entry from KNOWN_STATUS_DRIFT"
        );
    }
}
