//! Route handlers for the SHIFT proxy.

pub mod anthropic;
pub mod google;
pub mod health;
pub mod openai;
pub mod passthrough;

use crate::ProxyState;
use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
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
    body: Bytes,
) -> Response {
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let rewritten: Uri = format!("/v1/messages{}", query)
        .parse()
        .expect("/v1/messages is a valid URI");

    anthropic::anthropic_handler(state, rewritten, headers, body).await
}

/// Fallback handler for `POST /responses` (without the `/v1` prefix).
///
/// Codex CLI sets `openai_base_url = "http://localhost:8787"` and then sends
/// requests to `/responses` (the OpenAI Responses API). We rewrite to
/// `/v1/responses` so the OpenAI handler builds the correct upstream URL.
async fn responses_fallback_handler(
    state: State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let rewritten: Uri = format!("/v1/responses{}", query)
        .parse()
        .expect("/v1/responses is a valid URI");

    openai::openai_handler(state, rewritten, headers, body).await
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
        .route("/v1/responses", post(openai::openai_handler))
        // Fallback: bare paths without /v1 prefix (resilience for misconfigured clients)
        .route("/messages", post(messages_fallback_handler))
        .route("/responses", post(responses_fallback_handler))
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
