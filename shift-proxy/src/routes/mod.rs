//! Route handlers for the SHIFT proxy.

pub mod anthropic;
pub mod google;
pub mod health;
pub mod openai;
pub mod passthrough;

use crate::ProxyState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get, post};
use axum::Router;

/// Maximum request body size: 200 MB.
/// AI payloads with base64 images can be large (50MB+). This limit prevents
/// unbounded memory consumption from malicious clients while accommodating
/// legitimate multi-image payloads.
const MAX_BODY_SIZE: usize = 200 * 1024 * 1024;

/// Build the complete proxy router with all routes.
pub fn build_router(state: ProxyState) -> Router {
    Router::new()
        // Health and stats
        .route("/health", get(health::health_handler))
        .route("/stats", get(health::stats_handler))
        // Provider-specific routes (with optimization)
        .route("/v1/messages", post(anthropic::anthropic_handler))
        .route("/v1/chat/completions", post(openai::openai_handler))
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
