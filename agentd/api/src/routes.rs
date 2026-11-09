//! `/v1` router: handlers of implemented routes; the other table routes answer `404 not_implemented`.

use std::sync::Arc;

use axum::Router;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{ConnectInfo, Request, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Uri};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};

use crate::access::{self, Denied};
use crate::error::ApiError;
use crate::layers;
use crate::peer::PeerInfo;
use crate::state::ApiState;
use crate::{
    actions, chat, connect, events, export, files, health, links, logout, modules, mutation_gate,
    permissions, quotas, requests, search, secrets, settings, tasks, tokens, tool_results, tree,
    triggers, tunnels, ws,
};

pub(crate) fn router(state: Arc<ApiState>) -> Router {
    let [sessions, session, thread, read] = tree::session_routes();
    let chat = chat::routes();
    let [ev_list, ev_read, ev_subtypes, ev_one] = events::routes();
    let task = tasks::routes();
    let (trig_all, trig_one) = triggers::routes();
    let (tun_all, tun_one) = tunnels::routes();
    let (mod_all, mod_one) = modules::routes();
    let router = Router::new()
        .route("/v1/health", get(health::handler))
        .route("/v1/metrics", get(health::metrics))
        .route("/v1/tokens", get(tokens::list).post(tokens::create))
        .route("/v1/tokens/{id}", delete(tokens::revoke))
        .route(
            "/v1/settings/{page}",
            get(settings::get_page).patch(settings::patch_page),
        )
        .route("/v1/sessions", sessions)
        .route("/v1/sessions/{id}", session)
        .route("/v1/sessions/{id}/threads", thread)
        .route("/v1/sessions/{id}/read", read)
        .route("/v1/sessions/{id}/messages", chat.messages)
        .route("/v1/sessions/{id}/clear", chat.clear)
        .route("/v1/sessions/{id}/stop", chat.stop)
        .route("/v1/sessions/{id}/tool-results", tool_results::route())
        .route("/v1/sessions/{id}/bookmarks", links::bookmarks_route())
        .route("/v1/browser/quicklinks", links::quicklinks_route())
        .route("/v1/sessions/{id}/attachments", chat.upload)
        .route("/v1/attachments/{id}", chat.download)
        .route("/v1/sessions/{id}/tasks", task.session)
        .route("/v1/tasks", task.all)
        .route("/v1/tasks/{id}", task.one)
        .route("/v1/tasks/{id}/runs", task.runs)
        .route("/v1/tasks/{id}/run", task.run)
        .route("/v1/triggers", trig_all)
        .route("/v1/triggers/{id}", trig_one)
        .route("/v1/sessions/{id}/export", export::route())
        .route("/v1/logout", logout::route())
        .route("/v1/modules", mod_all)
        .route("/v1/modules/{id}", mod_one)
        .route("/v1/modules/{id}/token", modules::token_route())
        .route("/v1/tunnels", tun_all)
        .route("/v1/tunnels/{id}", tun_one)
        .route("/v1/search", search::route())
        .route("/v1/files", files::list_route())
        .route("/v1/files/content", files::content_route())
        .route("/v1/quotas", quotas::route())
        .route("/v1/events", ev_list)
        .route("/v1/events/read", ev_read)
        .route("/v1/events/subtypes", ev_subtypes)
        .route("/v1/actions", actions::route())
        .route("/v1/events/{id}/read", ev_one)
        .route("/v1/permission-requests/{id}", requests::get_route())
        .route(
            "/v1/permission-requests/{id}/decision",
            requests::decision_route(),
        )
        .route("/v1/permissions", permissions::list_route())
        .route(
            "/v1/permissions/{scope}/{scope_id}/{action}",
            permissions::put_route(),
        )
        .route("/v1/folders", tree::folder_routes().0)
        .route("/v1/folders/{id}", tree::folder_routes().1)
        .route("/v1/personas", tree::persona_routes().0)
        .route("/v1/personas/{id}", tree::persona_routes().1)
        .route("/v1/accounts", tree::account_routes().0)
        .route("/v1/accounts/{id}", tree::account_routes().1)
        .route("/v1/accounts/{id}/connect", connect::route())
        .route("/v1/secrets", secrets::routes().0)
        .route("/v1/secrets/{ref}", secrets::routes().1)
        .route(layers::WS_PATH, get(ws_upgrade))
        .fallback(fallback);
    #[cfg(feature = "test-support")]
    let router = router
        .route(
            "/v1/_test/advance",
            axum::routing::post(crate::testsupport::advance),
        )
        .route(
            "/v1/_test/observer",
            axum::routing::post(crate::testsupport::observer),
        )
        .route(
            "/v1/_test/indicator",
            axum::routing::post(crate::testsupport::indicator),
        )
        .route(
            "/v1/_test/ingest",
            axum::routing::post(crate::testsupport::ingest),
        )
        .route(
            "/v1/_test/channel",
            axum::routing::post(crate::testsupport::channel),
        );
    router
        .layer(from_fn_with_state(state.clone(), mutation_gate::layer))
        .layer(from_fn_with_state(state.clone(), access::guard))
        .layer(from_fn_with_state(state.clone(), crate::web::cors))
        .layer(from_fn_with_state(state.clone(), layers::envelope))
        .with_state(state)
}

/// Everything without a handler lands here: a table route is "not implemented", otherwise "not found".
async fn fallback(req: Request) -> ApiError {
    if req.extensions().get::<access::MatchedRoute>().is_some() {
        ApiError::not_implemented()
    } else {
        ApiError::not_found()
    }
}

/// `X-Api-Major` is optional; if present, it must match the core's major version.
fn api_major_supported(headers: &HeaderMap) -> bool {
    headers.get("x-api-major").is_none_or(|v| {
        v.to_str().ok().and_then(|s| s.trim().parse::<u32>().ok()) == Some(health::API_MAJOR)
    })
}

async fn ws_upgrade(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(peer): ConnectInfo<PeerInfo>,
    headers: HeaderMap,
    uri: Uri,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let closed = peer.closed.clone();
    // The browser requires the server to pick one of the offered subprotocols; the token value is not returned.
    let upgrade = upgrade.map(|u| u.protocols([crate::web::WS_PROTOCOL]));
    if !layers::origin_host_allowed(&state, &peer, &headers) {
        return match upgrade {
            Ok(u) => u.on_upgrade(move |s| ws::close_with(s, ws::CLOSE_FORBIDDEN)),
            Err(_) => ApiError::forbidden_origin().into_response(),
        };
    }
    // While the core is migrating or starting, WS is not accepted (docs/reference/api.md 2.1).
    let Some(services) = state.services() else {
        return state.not_ready().into_response();
    };
    let who = access::identify(&state, &services.auth, &peer, &headers, uri.query()).await;
    match (who, upgrade) {
        (Ok(principal), Ok(u)) => {
            let u = u.max_message_size(ws::MAX_CLIENT_MESSAGE);
            // The client names its API major version: the core does not serve another one (docs/reference/api.md 7, code 4426).
            if !api_major_supported(&headers) {
                return u.on_upgrade(move |s| ws::close_with(s, ws::CLOSE_VERSION));
            }
            // A token with a method list (agent, module) may subscribe only if the list contains `GET /ws`.
            if principal
                .allowlist
                .as_ref()
                .is_some_and(|l| !l.iter().any(|m| m == "GET /ws"))
            {
                tracing::warn!(
                    scope = "auth",
                    event = "forbidden_method",
                    token_id = principal.token_id,
                    route = "/ws",
                    "WS not in token list"
                );
                return ApiError::forbidden_method().into_response();
            }
            let state = state.clone();
            u.on_upgrade(move |s| ws::run(s, state, principal, closed))
        }
        (Ok(_), Err(_)) => ApiError::bad_request("A WebSocket upgrade is required").into_response(),
        (Err(Denied::Unauthorized(_)), Ok(u)) => {
            u.on_upgrade(move |s| ws::close_with(s, ws::CLOSE_UNAUTHORIZED))
        }
        (Err(Denied::Unauthorized(e) | Denied::Reply(e)), _) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table route without a handler answers `404 not_implemented`, an unknown path `404` without a reason.
    #[tokio::test]
    async fn fallback_distinguishes_unimplemented_from_unknown() {
        let mut known = Request::new(axum::body::Body::empty());
        known
            .extensions_mut()
            .insert(access::MatchedRoute(&crate::table::ROUTES[0]));
        let e = fallback(known).await;
        assert_eq!(e.status, 404);
        assert_eq!(e.body("t")["error"]["details"]["reason"], "not_implemented");
        let e = fallback(Request::new(axum::body::Body::empty())).await;
        assert_eq!(e.status, 404);
        assert!(e.body("t")["error"]["details"]["reason"].is_null());
    }
}
