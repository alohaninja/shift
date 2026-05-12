//! Google route handler — POST /v1beta/models/* and /v1/models/*
//!
//! Pure passthrough — no optimization. Google payload parsing is not yet
//! implemented in shift-core (matches the TS proxy behavior).
//! Query params are preserved (may contain API keys — redacted in logs).

use crate::body::extract_body;
use crate::forward::forward_request;
use crate::ProxyState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// POST /v1beta/models/* or /v1/models/* — forward to Google unchanged.
pub async fn google_handler(
    State(state): State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = match extract_body(&headers, body) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    };
    let base_url = &state.config.providers.google;
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let target_url = format!("{}{}{}", base_url, uri.path(), query);

    if state.config.verbose {
        // Redact query params from log output (may contain API keys)
        tracing::info!("Google: passthrough → {}{}", base_url, uri.path());
    }

    forward_request(
        &state.http_client,
        "POST",
        &target_url,
        &headers,
        Some(body),
    )
    .await
}
