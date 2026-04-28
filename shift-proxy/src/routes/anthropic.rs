//! Anthropic route handler — POST /v1/messages
//!
//! Intercepts Anthropic API requests, runs the SHIFT optimization pipeline
//! on the payload (extracting and transforming images), records stats with
//! full per-image token savings, and forwards to the real Anthropic API.

use crate::forward::forward_request;
use crate::ProxyState;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;

/// POST /v1/messages — optimize and forward to Anthropic.
pub async fn anthropic_handler(
    State(state): State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let config = state.config.shift_config("anthropic");
    let base_url = &state.config.providers.anthropic;
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let target_url = format!("{}{}{}", base_url, uri.path(), query);

    // Run SHIFT optimization pipeline
    let start = std::time::Instant::now();
    let (final_body, optimized) = match optimize_payload(&body, &config) {
        Some((transformed_json, report)) => {
            let duration_ms = start.elapsed().as_millis() as u64;

            // Record session stats (in-memory)
            state.session.record(&report);

            // Record persistent stats with FULL token savings
            let record =
                shift_preflight::stats::record_from_report(&report, "anthropic", duration_ms);
            if let Err(e) = shift_preflight::stats::record_run(&record, None) {
                tracing::warn!("failed to save stats: {}", e);
            }

            if state.config.verbose {
                let saved = report.original_size.saturating_sub(report.transformed_size);
                if saved > 0 {
                    tracing::info!(
                        "Anthropic: saved {:.1}KB ({} tokens)",
                        saved as f64 / 1024.0,
                        report.token_savings.anthropic_saved(),
                    );
                }
            }

            (transformed_json, true)
        }
        None => (body, false),
    };

    if state.config.verbose && !optimized {
        tracing::debug!("Anthropic: no optimization applied (passthrough)");
    }

    forward_request(
        &state.http_client,
        "POST",
        &target_url,
        &headers,
        Some(final_body),
    )
    .await
}

/// Run the SHIFT pipeline on a JSON payload.
/// Returns the transformed JSON string and report, or None if no optimization
/// was needed or an error occurred (fail-safe: passthrough on error).
fn optimize_payload(
    body: &str,
    config: &shift_preflight::ShiftConfig,
) -> Option<(String, shift_preflight::Report)> {
    // Parse the payload
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to parse payload as JSON: {}", e);
            return None;
        }
    };

    // Run the pipeline
    let (result, report) = match shift_preflight::process(&payload, config) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("SHIFT pipeline error: {}", e);
            return None;
        }
    };

    // Only return if something actually changed
    if !report.has_changes() {
        return None;
    }

    // Serialize back to JSON (compact, not pretty — this is a wire payload)
    match serde_json::to_string(&result) {
        Ok(json) => Some((json, report)),
        Err(e) => {
            tracing::warn!("failed to serialize optimized payload: {}", e);
            None
        }
    }
}
