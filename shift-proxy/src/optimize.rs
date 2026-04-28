//! Shared image optimization logic for provider route handlers.
//!
//! Extracted to avoid duplication between Anthropic and OpenAI routes.
//! The optimization runs on a blocking thread to avoid starving the
//! tokio event loop during CPU-intensive image operations.

use shift_preflight::{Report, ShiftConfig};

/// Run the SHIFT pipeline on a JSON payload.
///
/// Returns the transformed JSON string and report, or None if no optimization
/// was needed or an error occurred (fail-safe: passthrough on error).
///
/// This function is synchronous (CPU-bound image operations) and MUST be
/// called from `tokio::task::spawn_blocking`.
pub fn optimize_payload(body: &str, config: &ShiftConfig) -> Option<(String, Report)> {
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
