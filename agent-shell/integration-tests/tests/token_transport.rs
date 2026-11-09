//! Token on the transport: address, WS 4401, attempt limit 429, SO_PEERCRED.

mod common;

use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use common::{error_of, first_ws_message, http, quiet, send, start, unix};
use core_client::http::Request;
use core_client::ws::{WsConn, WsIn};
use core_client::{ClientError, Token};
use fake_agentd::{Config, Kind, Scope, TokenSpec};
use serde_json::{Value, json};

#[tokio::test]
async fn query_token_rejected_401() {
    let d = quiet().await;
    let c = http(&d, None);
    let t = d.admin_token().to_string();
    for query in [
        format!("token={t}"),
        format!("access_token={t}"),
        format!("x=1&token={t}"),
        "%74oken=abc".to_string(),
        "token=".to_string(),
    ] {
        let r = send(
            &c,
            Request::get(&format!("/v1/settings/theme?{query}")).anonymous(),
        )
        .await;
        let (code, details) = error_of(&r);
        assert_eq!(
            (r.status, code.as_str()),
            (401, "unauthorized"),
            "?{}",
            query.split('=').next().unwrap_or("")
        );
        assert_eq!(details["reason"], json!("token_in_url"));
        // even with a valid header a token in the address is rejected
        let r = send(
            &http(&d, Some(&t)),
            Request::get(&format!("/v1/settings/theme?{query}")),
        )
        .await;
        assert_eq!(
            r.status, 401,
            "a header does not justify a token in the address"
        );
    }
    assert_eq!(
        send(
            &http(&d, Some(&t)),
            Request::get("/v1/settings/theme?lang=ru")
        )
        .await
        .status,
        200,
        "a harmless parameter accepted"
    );
    // the token in the address did not reach the mock's record
    assert!(
        d.requests().iter().all(|r| !r.path.contains("token")),
        "the path in the log without a query string"
    );
}

#[tokio::test]
async fn ws_close_4401() {
    let d = quiet().await;
    // a revoked token
    let issued = d.issue(TokenSpec::new(Scope::Read, Kind::Test));
    let Ok(issued) = issued else {
        panic!("{issued:?}")
    };
    d.revoke(issued.id);
    let tok = Token::new(&issued.token);
    let mut ws = match WsConn::connect(&unix(&d), Some(&tok), "/v1/ws").await {
        Ok(w) => w,
        Err(e) => panic!("the handshake must pass, the refusal comes as a close code: {e}"),
    };
    assert_eq!(
        first_ws_message(&mut ws).await,
        Some(WsIn::Close(Some(4401))),
        "revoked token"
    );
    // a token in the address
    let admin = Token::new(d.admin_token());
    let mut ws = match WsConn::connect(
        &unix(&d),
        Some(&admin),
        &format!("/v1/ws?token={}", d.admin_token()),
    )
    .await
    {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(
        first_ws_message(&mut ws).await,
        Some(WsIn::Close(Some(4401))),
        "token in the address"
    );
    // revocation with an open connection
    let live = d.issue(TokenSpec::new(Scope::Read, Kind::Test));
    let Ok(live) = live else { panic!("{live:?}") };
    let live_tok = Token::new(&live.token);
    let mut ws = match WsConn::connect(&unix(&d), Some(&live_tok), "/v1/ws").await {
        Ok(w) => w,
        Err(e) => panic!("{e}"),
    };
    let revoke = http(&d, Some(d.admin_token())).revoke_token(live.id).await;
    assert!(revoke.is_ok(), "{revoke:?}");
    assert_eq!(
        first_ws_message(&mut ws).await,
        Some(WsIn::Close(Some(4401))),
        "revocation with an open connection"
    );
    // no token at the handshake: HTTP 401
    let anon = WsConn::connect(&unix(&d), None, "/v1/ws").await;
    assert!(
        matches!(&anon, Err(ClientError::Api(e)) if e.status == 401),
        "without a token: {:?}",
        anon.err()
    );
}

#[tokio::test]
async fn rate_limit_429_5_per_min() {
    let d = start(Config::temp().no_ticker().manual_clock()).await;
    let valid = http(&d, Some(d.admin_token()));
    for i in 0..5 {
        let r = send(
            &http(&d, Some(&format!("agt_wrong{i:038}"))),
            Request::get("/v1/settings/theme"),
        )
        .await;
        assert_eq!(r.status, 401, "wrong token #{}", i + 1);
    }
    let sixth = send(
        &http(&d, Some("agt_wrong-sixth")),
        Request::get("/v1/settings/theme"),
    )
    .await;
    assert_eq!(sixth.status, 429, "sixth within a minute");
    assert_eq!(error_of(&sixth).0, "rate_limited");
    let retry: u64 = sixth
        .header("retry-after")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(
        (1..=60).contains(&retry),
        "Retry-After in seconds, at least 1: {retry}"
    );
    let body: Value = serde_json::from_slice(&sixth.body).unwrap_or(Value::Null);
    assert!(
        body["error"]["request_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "request_id in the body"
    );
    assert_eq!(
        send(&valid, Request::get("/v1/settings/theme"))
            .await
            .status,
        429,
        "the pause applies to a correct token from the same address too"
    );
    d.advance(Duration::from_secs(61));
    assert_eq!(
        send(&valid, Request::get("/v1/settings/theme"))
            .await
            .status,
        200,
        "after a minute the pause is lifted"
    );
    // a missing token and a token in the address do not count as attempts
    for _ in 0..8 {
        send(
            &http(&d, None),
            Request::get("/v1/settings/theme").anonymous(),
        )
        .await;
    }
    assert_eq!(
        send(&valid, Request::get("/v1/settings/theme"))
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn peercred_same_uid_not_distinguished() {
    let d = quiet().await;
    let sock_uid = std::fs::metadata(d.socket_path())
        .map(|m| m.uid())
        .unwrap_or(u32::MAX);
    let r = send(&http(&d, None), Request::get("/v1/tokens").anonymous()).await;
    assert_eq!(
        r.status, 401,
        "uid matches, but without a token there is no access"
    );
    let rec = d.requests().into_iter().last();
    assert_eq!(
        rec.as_ref().and_then(|r| r.peer_uid),
        Some(sock_uid),
        "SO_PEERCRED read and equals the socket owner's uid"
    );
    let foreign_file = d.dir().join("someone-elses.token");
    let wrote = std::fs::write(
        &foreign_file,
        "agt_00000000000000000000000000000000000000000000",
    );
    assert!(wrote.is_ok());
    let foreign = std::fs::read_to_string(&foreign_file).unwrap_or_default();
    let r = send(&http(&d, Some(foreign.trim())), Request::get("/v1/tokens")).await;
    assert_eq!(
        r.status, 401,
        "another token file of the same uid does not let in"
    );
}
