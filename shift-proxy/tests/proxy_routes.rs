//! Integration tests for the SHIFT proxy routes.
//!
//! All tests use a local mock upstream server — no real API calls.
//! The mock echoes back request metadata (path, method, headers, body)
//! so we can verify the proxy routes, forwards, and transforms correctly.

use axum::body::Body;
use axum::extract::State as AxumState;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::Json;
use axum::routing::{any, get};
use axum::Router;
use http_body_util::BodyExt;
use shift_proxy::state::ProviderUrls;
use shift_proxy::{create_app, ProxyConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt; // for `oneshot`

// ── Mock upstream server ─────────────────────────────────────────────

/// Shared state for the mock upstream — counts requests.
#[derive(Clone, Default)]
struct MockState {
    request_count: Arc<AtomicU64>,
}

/// Start a mock upstream HTTP server on a random port.
/// Returns the base URL (e.g., "http://127.0.0.1:12345").
async fn start_mock_upstream() -> (String, MockState) {
    let state = MockState::default();

    let app = Router::new()
        .route("/health", get(mock_health))
        .fallback(any(mock_echo))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, state)
}

/// Mock health endpoint.
async fn mock_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "service": "mock-upstream"}))
}

/// Mock catch-all: echoes back request metadata as JSON.
/// This lets tests verify that the proxy forwarded correctly.
async fn mock_echo(
    AxumState(state): AxumState<MockState>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: String,
) -> Json<serde_json::Value> {
    state.request_count.fetch_add(1, Ordering::Relaxed);

    // Collect headers into a map (skip pseudo-headers)
    let header_map: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                serde_json::Value::String(v.to_str().unwrap_or("").to_string()),
            )
        })
        .collect();

    Json(serde_json::json!({
        "method": method.as_str(),
        "path": uri.path(),
        "query": uri.query().unwrap_or(""),
        "headers": header_map,
        "body": body,
    }))
}

/// Create a ProxyConfig that points all providers at the mock upstream.
fn test_config_with_mock(mock_url: &str) -> ProxyConfig {
    ProxyConfig {
        port: 0,
        verbose: false,
        providers: ProviderUrls {
            anthropic: mock_url.to_string(),
            openai: mock_url.to_string(),
            google: mock_url.to_string(),
        },
        ..ProxyConfig::default()
    }
}

/// Helper: extract JSON body from an axum response.
async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

// ── Health endpoint ──────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_with_service_identity() {
    let app = create_app(ProxyConfig::default());

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
    let json = json_body(response).await;

    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "@shift-preflight/runtime proxy");
    assert!(json["version"].is_string());
    assert!(!json["version"].as_str().unwrap().is_empty());
}

// ── Stats endpoint ───────────────────────────────────────────────────

#[tokio::test]
async fn stats_returns_session_stats() {
    let app = create_app(ProxyConfig::default());

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
    let json = json_body(response).await;

    assert!(json["totalRequests"].is_number());
    assert!(json["totalImages"].is_number());
    assert!(json["totalImagesModified"].is_number());
    assert!(json["totalBytesSaved"].is_number());
    assert!(json["tokenSavings"].is_object());
}

// ── 404 for unknown routes ───────────────────────────────────────────

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let app = create_app(ProxyConfig::default());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/unknown/endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Anthropic route — forwards to mock upstream ──────────────────────

#[tokio::test]
async fn anthropic_route_forwards_to_upstream() {
    let (mock_url, mock_state) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

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
                .header("x-api-key", "sk-ant-test123")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(serde_json::to_string(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;

    // Verify the mock received the request at the correct path
    assert_eq!(json["path"], "/v1/messages");
    assert_eq!(json["method"], "POST");

    // Verify auth headers were forwarded
    assert_eq!(json["headers"]["x-api-key"], "sk-ant-test123");
    assert_eq!(json["headers"]["anthropic-version"], "2023-06-01");

    // Verify the body was forwarded (text-only payload — no optimization needed)
    let forwarded_body: serde_json::Value =
        serde_json::from_str(json["body"].as_str().unwrap()).unwrap();
    assert_eq!(forwarded_body["model"], "claude-sonnet-4-20250514");
    assert_eq!(forwarded_body["messages"][0]["content"], "test");

    // Verify mock received exactly 1 request
    assert_eq!(mock_state.request_count.load(Ordering::Relaxed), 1);
}

// ── OpenAI route — forwards to mock upstream ─────────────────────────

#[tokio::test]
async fn openai_route_forwards_to_upstream() {
    let (mock_url, mock_state) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

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
                .header("authorization", "Bearer sk-test456")
                .body(Body::from(serde_json::to_string(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;

    assert_eq!(json["path"], "/v1/chat/completions");
    assert_eq!(json["method"], "POST");
    assert_eq!(json["headers"]["authorization"], "Bearer sk-test456");

    let forwarded_body: serde_json::Value =
        serde_json::from_str(json["body"].as_str().unwrap()).unwrap();
    assert_eq!(forwarded_body["model"], "gpt-4o");

    assert_eq!(mock_state.request_count.load(Ordering::Relaxed), 1);
}

// ── Google route — forwards with query params preserved ──────────────

#[tokio::test]
async fn google_route_forwards_with_query_params() {
    let (mock_url, _) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-pro:generateContent?key=test-key-789")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"contents": [{"parts": [{"text": "hi"}]}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;

    // Verify path and query params are forwarded correctly
    assert_eq!(
        json["path"],
        "/v1beta/models/gemini-2.5-pro:generateContent"
    );
    assert_eq!(json["query"], "key=test-key-789");
}

// ── Passthrough — forwards to correct provider ───────────────────────

#[tokio::test]
async fn passthrough_forwards_anthropic_subpaths() {
    let (mock_url, mock_state) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

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

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["path"], "/v1/messages/batches");
    assert_eq!(mock_state.request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn passthrough_returns_404_for_unknown_provider() {
    let (mock_url, _) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

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

// ── GET method passthrough ───────────────────────────────────────────

#[tokio::test]
async fn get_request_forwarded_through_passthrough() {
    let (mock_url, _) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

    // GET /v1/models should be forwarded to OpenAI (via passthrough)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("authorization", "Bearer sk-test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["path"], "/v1/models");
    assert_eq!(json["method"], "GET");
    assert_eq!(json["headers"]["authorization"], "Bearer sk-test");
}

// ── Auth headers NOT stripped ────────────────────────────────────────

#[tokio::test]
async fn auth_headers_forwarded_to_upstream() {
    let (mock_url, _) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("x-api-key", "sk-ant-secret")
                .header("anthropic-version", "2023-06-01")
                .header("authorization", "Bearer also-present")
                .body(Body::from(r#"{"model":"claude-sonnet-4-20250514","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;

    assert_eq!(json["headers"]["x-api-key"], "sk-ant-secret");
    assert_eq!(json["headers"]["anthropic-version"], "2023-06-01");
    assert_eq!(json["headers"]["authorization"], "Bearer also-present");
}

// ── Host header stripped ─────────────────────────────────────────────

#[tokio::test]
async fn host_header_not_forwarded() {
    let (mock_url, _) = start_mock_upstream().await;
    let app = create_app(test_config_with_mock(&mock_url));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("host", "evil.example.com")
                .body(Body::from(r#"{"model":"claude-sonnet-4-20250514","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;

    // The original "host: evil.example.com" should have been stripped.
    // reqwest sets its own Host header from the target URL.
    let host = json["headers"]["host"].as_str().unwrap_or("");
    assert!(
        !host.contains("evil"),
        "original host header should be stripped, got: {}",
        host
    );
}

// ── Health endpoint backward compatibility ────────────────────────────

#[tokio::test]
async fn health_backward_compatible_with_opencode_plugin() {
    let app = create_app(ProxyConfig::default());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = json_body(response).await;

    // OpenCode plugin checks: body.service === "@shift-preflight/runtime proxy"
    assert_eq!(
        json["service"].as_str().unwrap(),
        "@shift-preflight/runtime proxy"
    );

    // OpenCode plugin checks: body.version exists
    assert!(json.get("version").is_some());
    assert!(!json["version"].as_str().unwrap().is_empty());
}
