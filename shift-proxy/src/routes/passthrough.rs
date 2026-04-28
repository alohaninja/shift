//! Catch-all passthrough handler.
//!
//! Forwards requests to the upstream provider detected from the request
//! path. Used for routes not explicitly matched by the provider-specific
//! handlers (e.g., OpenAI batch endpoints, Anthropic beta paths, GET
//! /v1/models, etc.).

use crate::forward::forward_request;
use crate::ProxyState;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Catch-all handler — detect provider from path and forward unchanged.
/// Handles all HTTP methods (GET, POST, PUT, PATCH, DELETE).
pub async fn passthrough_handler(
    State(state): State<ProxyState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let path = uri.path();
    let provider = detect_provider_from_route(path);

    let base_url = match provider {
        Some("anthropic") => &state.config.providers.anthropic,
        Some("openai") => &state.config.providers.openai,
        Some("google") => &state.config.providers.google,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({
                    "error": "Unknown route — cannot determine upstream provider"
                })),
            )
                .into_response();
        }
    };

    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let target_url = format!("{}{}{}", base_url, path, query);

    if state.config.verbose {
        tracing::info!("Passthrough: {} {} → {}{}", method, path, base_url, path);
    }

    // For methods without a body (GET, HEAD), don't forward one.
    let has_body = !matches!(method, Method::GET | Method::HEAD);
    let body = if has_body { Some(body) } else { None };

    forward_request(
        &state.http_client,
        method.as_str(),
        &target_url,
        &headers,
        body,
    )
    .await
}

/// Detect which provider a route path belongs to.
fn detect_provider_from_route(path: &str) -> Option<&'static str> {
    if path.starts_with("/v1/messages") {
        Some("anthropic")
    } else if path.starts_with("/v1/chat/") || path.starts_with("/v1/embeddings") {
        Some("openai")
    } else if path.starts_with("/v1beta/") || path.starts_with("/v1/models/gemini") {
        Some("google")
    } else if path.starts_with("/v1/") {
        // Default to OpenAI for /v1/* paths (most common)
        Some("openai")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_anthropic() {
        assert_eq!(
            detect_provider_from_route("/v1/messages"),
            Some("anthropic")
        );
        assert_eq!(
            detect_provider_from_route("/v1/messages/batches"),
            Some("anthropic")
        );
    }

    #[test]
    fn detect_openai() {
        assert_eq!(
            detect_provider_from_route("/v1/chat/completions"),
            Some("openai")
        );
        assert_eq!(detect_provider_from_route("/v1/embeddings"), Some("openai"));
    }

    #[test]
    fn detect_google() {
        assert_eq!(
            detect_provider_from_route("/v1beta/models/gemini-2.5-pro:generateContent"),
            Some("google")
        );
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(detect_provider_from_route("/unknown"), None);
    }
}
