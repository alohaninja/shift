//! Health and stats endpoints.

use crate::ProxyState;
use axum::extract::State;
use axum::response::Json;
use serde_json::json;

/// GET /health — proxy health check.
///
/// Returns a JSON object with service identity and version.
/// The service string MUST match `HEALTH_SERVICE_ID` checked by
/// the OpenCode plugin and `shift-ai proxy status`.
pub async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "@shift-preflight/runtime proxy",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /stats — session statistics.
pub async fn stats_handler(State(state): State<ProxyState>) -> Json<serde_json::Value> {
    Json(state.session.to_json())
}
