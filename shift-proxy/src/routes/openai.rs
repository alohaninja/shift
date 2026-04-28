//! OpenAI route handler — POST /v1/chat/completions
//!
//! Intercepts OpenAI API requests, runs the SHIFT optimization pipeline
//! on the payload, records stats with full per-image token savings, and
//! forwards to the real OpenAI API.

use crate::forward::forward_request;
use crate::ProxyState;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;

/// POST /v1/chat/completions — optimize and forward to OpenAI.
pub async fn openai_handler(
    State(state): State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let config = state.config.shift_config("openai");
    let base_url = &state.config.providers.openai;
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
                shift_preflight::stats::record_from_report(&report, "openai", duration_ms);
            if let Err(e) = shift_preflight::stats::record_run(&record, None) {
                tracing::warn!("failed to save stats: {}", e);
            }

            if state.config.verbose {
                let saved = report.original_size.saturating_sub(report.transformed_size);
                if saved > 0 {
                    tracing::info!(
                        "OpenAI: saved {:.1}KB ({} tokens)",
                        saved as f64 / 1024.0,
                        report.token_savings.openai_saved(),
                    );
                }
            }

            (transformed_json, true)
        }
        None => (body, false),
    };

    if state.config.verbose && !optimized {
        tracing::debug!("OpenAI: no optimization applied (passthrough)");
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

/// Run the SHIFT pipeline on a JSON payload (same logic as Anthropic route).
fn optimize_payload(
    body: &str,
    config: &shift_preflight::ShiftConfig,
) -> Option<(String, shift_preflight::Report)> {
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to parse payload as JSON: {}", e);
            return None;
        }
    };

    let (result, report) = match shift_preflight::process(&payload, config) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("SHIFT pipeline error: {}", e);
            return None;
        }
    };

    if !report.has_changes() {
        return None;
    }

    match serde_json::to_string(&result) {
        Ok(json) => Some((json, report)),
        Err(e) => {
            tracing::warn!("failed to serialize optimized payload: {}", e);
            None
        }
    }
}
