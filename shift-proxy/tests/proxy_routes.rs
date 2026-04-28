//! Integration tests for the SHIFT proxy routes.
//!
//! These tests exercise the proxy router via axum's test utilities.
//! They do NOT make real network calls to upstream providers — they test
//! the proxy's internal routing, optimization pipeline, stats recording,
//! and header handling.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use shift_proxy::{create_app, ProxyConfig};
use tower::ServiceExt; // for `oneshot`

fn test_config() -> ProxyConfig {
    ProxyConfig {
        port: 0, // Not binding a real listener in tests
        verbose: false,
        ..ProxyConfig::default()
    }
}

// ── Health endpoint ──────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_with_service_identity() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "@shift-preflight/runtime proxy");
    assert!(json["version"].is_string());
    assert!(!json["version"].as_str().unwrap().is_empty());
}

// ── Stats endpoint ───────────────────────────────────────────────────

#[tokio::test]
async fn stats_returns_session_stats() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json["totalRequests"].is_number());
    assert!(json["totalImages"].is_number());
    assert!(json["totalImagesModified"].is_number());
    assert!(json["totalBytesSaved"].is_number());
    assert!(json["tokenSavings"].is_object());
    assert!(json["tokenSavings"]["openai_before"].is_number());
    assert!(json["tokenSavings"]["anthropic_before"].is_number());
}

// ── 404/405 for unknown GET routes ────────────────────────────────────

#[tokio::test]
async fn unknown_get_returns_not_found_or_method_not_allowed() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/unknown/endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // GET to unknown path: 404 (no route) or 405 (fallback only handles POST)
    let status = response.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "expected 404 or 405, got {}",
        status
    );
}

// ── Anthropic route reachability ──────────────────────────────────────
// The handler forwards to the real Anthropic API. We verify the route
// is matched by checking we get a response (not 404). The actual status
// depends on network — 401 (auth error) if reachable, 502 if not.

#[tokio::test]
async fn anthropic_route_is_matched() {
    let app = create_app(test_config());

    let payload = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "test"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("x-api-key", "sk-ant-test")
                .body(Body::from(serde_json::to_string(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Route matched — we get an upstream response (401 auth error or 502 unreachable).
    // Either is fine; NOT 404 proves the route matched.
    let status = response.status();
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "route should be matched, got 404"
    );
}

// ── OpenAI route reachability ─────────────────────────────────────────

#[tokio::test]
async fn openai_route_is_matched() {
    let app = create_app(test_config());

    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "test"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer sk-test")
                .body(Body::from(serde_json::to_string(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "route should be matched, got 404"
    );
}

// ── Google route reachability ─────────────────────────────────────────

#[tokio::test]
async fn google_route_is_matched() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-pro:generateContent?key=test-key")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"contents": []}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "route should be matched, got 404"
    );
}

// ── Passthrough catch-all ─────────────────────────────────────────────

#[tokio::test]
async fn passthrough_catches_known_v1_post() {
    let app = create_app(test_config());

    // POST to a /v1/messages subpath (not exact match) — should be caught
    // by the fallback, which detects "anthropic" from the path prefix.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/batches")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"test": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should NOT be 404 — the passthrough should catch /v1/messages/* paths
    let status = response.status();
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "passthrough should catch /v1/messages/* routes, got 404"
    );
}

#[tokio::test]
async fn passthrough_returns_404_for_unknown_provider() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/unknown/path")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"test": true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Health endpoint backward compatibility ────────────────────────────
// The OpenCode plugin checks service === "@shift-preflight/runtime proxy"
// and version field exists. Verify we match exactly.

#[tokio::test]
async fn health_backward_compatible_with_opencode_plugin() {
    let app = create_app(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // OpenCode plugin checks: body.service === "@shift-preflight/runtime proxy"
    assert_eq!(
        json["service"].as_str().unwrap(),
        "@shift-preflight/runtime proxy"
    );

    // OpenCode plugin checks: body.version exists
    assert!(json.get("version").is_some());
    assert!(json["version"].as_str().unwrap().len() > 0);
}
