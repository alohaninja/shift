//! OpenAI route handler — POST /v1/chat/completions
//!
//! Intercepts OpenAI API requests, runs the SHIFT optimization pipeline
//! on the payload, records stats with full per-image token savings, and
//! forwards to the real OpenAI API.
//!
//! The CPU-intensive optimization runs on a blocking thread to avoid
//! starving the tokio event loop.

use crate::forward::forward_request;
use crate::optimize::optimize_payload;
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

    // Run SHIFT optimization pipeline on a blocking thread to avoid
    // starving the async runtime during CPU-intensive image operations.
    let start = std::time::Instant::now();
    let body_clone = body.clone();
    let optimization_result =
        tokio::task::spawn_blocking(move || optimize_payload(&body_clone, &config)).await;

    let (final_body, optimized) = match optimization_result {
        Ok(Some((transformed_json, report))) => {
            let duration_ms = start.elapsed().as_millis() as u64;

            // Record session stats (in-memory)
            state.session.record(&report);

            // Record persistent stats with FULL token savings
            let record = shift_preflight::stats::record_from_report(&report, "openai", duration_ms);
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
        Ok(None) | Err(_) => (body, false),
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
