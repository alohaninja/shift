//! Request body extraction with transparent decompression.
//!
//! Clients like Codex CLI send gzip-compressed request bodies with
//! `Content-Encoding: gzip`. Axum's `String` extractor rejects these
//! because raw gzip bytes aren't valid UTF-8. This module extracts the
//! raw `Bytes`, decompresses if needed, and converts to a UTF-8 string.

use axum::body::Bytes;
use axum::http::HeaderMap;
use flate2::read::GzDecoder;
use std::io::Read;

/// Extract the request body as a UTF-8 string, decompressing if the
/// `Content-Encoding` header indicates compression.
///
/// Supported encodings:
/// - `gzip` — decompressed via flate2
/// - (none / identity) — passed through as-is
pub fn extract_body(headers: &HeaderMap, raw: Bytes) -> Result<String, String> {
    let encoding = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let bytes = if encoding.eq_ignore_ascii_case("gzip") {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut decoded = Vec::new();
        decoder
            .read_to_end(&mut decoded)
            .map_err(|e| format!("gzip decode error: {e}"))?;
        decoded
    } else {
        raw.to_vec()
    };

    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_extract_plain_body() {
        let headers = HeaderMap::new();
        let body = Bytes::from(r#"{"model":"gpt-4o"}"#);
        let result = extract_body(&headers, body).unwrap();
        assert_eq!(result, r#"{"model":"gpt-4o"}"#);
    }

    #[test]
    fn test_extract_gzip_body() {
        let original = r#"{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hello"}]}"#;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "gzip".parse().unwrap());

        let result = extract_body(&headers, Bytes::from(compressed)).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_extract_gzip_body_case_insensitive() {
        let original = "hello world";

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "Gzip".parse().unwrap());

        let result = extract_body(&headers, Bytes::from(compressed)).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_extract_invalid_utf8() {
        let headers = HeaderMap::new();
        // Invalid UTF-8 sequence
        let body = Bytes::from(vec![0xff, 0xfe, 0xfd]);
        let result = extract_body(&headers, body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid UTF-8"));
    }

    #[test]
    fn test_extract_invalid_gzip() {
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "gzip".parse().unwrap());
        // Not valid gzip data
        let body = Bytes::from(vec![0x00, 0x01, 0x02, 0x03]);
        let result = extract_body(&headers, body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gzip decode error"));
    }
}
