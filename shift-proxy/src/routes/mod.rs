//! Route handlers for the SHIFT proxy.

pub mod anthropic;
pub mod google;
pub mod health;
pub mod openai;
pub mod passthrough;

use crate::ProxyState;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get, post};
use axum::Router;

/// Maximum request body size: 200 MB.
/// AI payloads with base64 images can be large (50MB+). This limit prevents
/// unbounded memory consumption from malicious clients while accommodating
/// legitimate multi-image payloads.
const MAX_BODY_SIZE: usize = 200 * 1024 * 1024;

/// Fallback handler for `POST /messages` (without the `/v1` prefix).
///
/// Some clients (e.g. OpenCode with a misconfigured `baseURL` that omits `/v1`)
/// send requests to `/messages` instead of `/v1/messages`. Rather than returning
/// a 404 "Unknown route" error, we rewrite the URI to `/v1/messages` and delegate
/// to the standard Anthropic handler.
async fn messages_fallback_handler(
    state: State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Rewrite /messages → /v1/messages so the Anthropic handler builds the
    // correct upstream URL (https://api.anthropic.com/v1/messages).
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let rewritten: Uri = format!("/v1/messages{}", query)
        .parse()
        .expect("/v1/messages is a valid URI");

    anthropic::anthropic_handler(state, rewritten, headers, body).await
}

/// Build the complete proxy router with all routes.
pub fn build_router(state: ProxyState) -> Router {
    Router::new()
        // Health and stats
        .route("/health", get(health::health_handler))
        .route("/stats", get(health::stats_handler))
        // Provider-specific routes (with optimization)
        .route("/v1/messages", post(anthropic::anthropic_handler))
        .route("/v1/chat/completions", post(openai::openai_handler))
        // Fallback: /messages → /v1/messages (resilience for misconfigured clients)
        .route("/messages", post(messages_fallback_handler))
        // Google routes (passthrough only)
        .route("/v1beta/models/{*path}", post(google::google_handler))
        .route("/v1/models/{*path}", post(google::google_handler))
        // Catch-all passthrough for all HTTP methods (not just POST).
        // Some provider APIs use GET (list models), PUT, DELETE, etc.
        .fallback(any(passthrough::passthrough_handler))
        // Explicit body size limit — prevents OOM from malicious payloads.
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(state)
}
