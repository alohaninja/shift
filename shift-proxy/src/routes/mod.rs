//! Route handlers for the SHIFT proxy.

pub mod anthropic;
pub mod google;
pub mod health;
pub mod openai;
pub mod passthrough;

use crate::ProxyState;
use axum::routing::{get, post};
use axum::Router;

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
        // Catch-all passthrough for other POST routes
        .fallback(post(passthrough::passthrough_handler))
        .with_state(state)
}
