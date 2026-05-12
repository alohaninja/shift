//! Forward requests to upstream provider APIs and stream responses back.
//!
//! Handles header forwarding (auth passthrough), hop-by-hop header stripping
//! (RFC 9110 §7.6.1), and transparent SSE/chunked response streaming.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use reqwest::Client;

/// Headers stripped from upstream responses before forwarding to the client.
///
/// - `content-encoding` / `content-length`: reqwest auto-decompresses response
///   bodies, so these are stale. Forwarding them causes double-decompression.
///   NOTE: The `gzip`, `brotli`, and `deflate` features MUST be enabled on reqwest
///   for this stripping to be correct. Without them, reqwest does NOT decompress,
///   and stripping content-encoding causes clients to receive raw compressed bytes.
/// - Hop-by-hop headers per RFC 9110 §7.6.1.
const STRIP_RESPONSE_HEADERS: &[&str] = &[
    "content-encoding",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
];

/// Headers stripped from the forwarded request.
///
/// - `host` / `content-length`: stale for the upstream connection.
/// - `accept-encoding`: let reqwest negotiate compression based on its enabled
///   decompression features (`gzip`, `brotli`, `deflate`). Forwarding the client's
///   header could request encodings reqwest can't decompress (e.g., `zstd`), which
///   would result in raw compressed bytes reaching the client after we strip
///   `content-encoding`.
/// - `content-encoding`: the proxy already decompressed the request body (see
///   `body::extract_body`), so telling the upstream it's still compressed would
///   cause a decode error on their end.
const STRIP_REQUEST_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "accept-encoding",
    "content-encoding",
];

/// Forward a request to an upstream URL, streaming the response back.
///
/// Auth headers (`authorization`, `x-api-key`, `anthropic-version`, `x-goog-api-key`)
/// pass through unchanged. The response body is streamed directly — SSE and
/// chunked responses are not buffered.
pub async fn forward_request(
    client: &Client,
    method: &str,
    target_url: &str,
    request_headers: &HeaderMap,
    body: Option<String>,
) -> Response {
    let forwarded_headers = forward_headers(request_headers);

    let mut req = match method.to_uppercase().as_str() {
        "POST" => client.post(target_url),
        "GET" => client.get(target_url),
        "PUT" => client.put(target_url),
        "DELETE" => client.delete(target_url),
        "PATCH" => client.patch(target_url),
        _ => client.post(target_url),
    };

    req = req.headers(forwarded_headers);

    if let Some(body) = body {
        req = req.body(body);
    }

    match req.send().await {
        Ok(upstream) => stream_response(upstream),
        Err(err) => {
            tracing::error!("upstream error: {}", err);
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": "Bad Gateway",
                    "detail": "Upstream provider unreachable"
                })),
            )
                .into_response()
        }
    }
}

/// Convert a reqwest Response into an axum Response, streaming the body
/// and stripping hop-by-hop headers.
fn stream_response(upstream: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);

    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        let name_str = name.as_str().to_lowercase();
        if STRIP_RESPONSE_HEADERS
            .iter()
            .any(|h| h == &name_str.as_str())
        {
            continue;
        }
        if let Ok(v) = HeaderValue::from_bytes(value.as_bytes()) {
            response_headers.insert(name.clone(), v);
        }
    }

    // Stream the response body directly without buffering.
    // This is critical for SSE (Anthropic/OpenAI streaming) to work correctly.
    let body = Body::from_stream(upstream.bytes_stream());

    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}

/// Forward request headers, stripping host/content-length but passing auth through.
fn forward_headers(original: &HeaderMap) -> HeaderMap {
    let strip: std::collections::HashSet<&str> = STRIP_REQUEST_HEADERS.iter().copied().collect();

    let mut result = HeaderMap::new();
    for (name, value) in original {
        let name_lower = name.as_str().to_lowercase();
        if !strip.contains(name_lower.as_str()) {
            result.insert(name.clone(), value.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

    #[test]
    fn forward_headers_strips_host_content_length_and_accept_encoding() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "example.com".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "42".parse().unwrap());
        headers.insert(header::ACCEPT_ENCODING, "gzip, br".parse().unwrap());
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        headers.insert(header::AUTHORIZATION, "Bearer sk-test".parse().unwrap());
        headers.insert("x-api-key", "sk-ant-test".parse().unwrap());
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());

        let result = forward_headers(&headers);

        assert!(result.get(header::HOST).is_none());
        assert!(result.get(header::CONTENT_LENGTH).is_none());
        assert!(
            result.get(header::ACCEPT_ENCODING).is_none(),
            "accept-encoding should be stripped so reqwest negotiates its own"
        );
        assert!(
            result.get(header::CONTENT_ENCODING).is_none(),
            "content-encoding should be stripped — body was already decompressed"
        );
        assert_eq!(result.get(header::AUTHORIZATION).unwrap(), "Bearer sk-test");
        assert_eq!(result.get("x-api-key").unwrap(), "sk-ant-test");
        assert_eq!(result.get("anthropic-version").unwrap(), "2023-06-01");
    }
}
