//! Shared proxy state and configuration.

use shift_preflight::{DriveMode, ShiftConfig, SvgMode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Proxy configuration.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub port: u16,
    pub mode: DriveMode,
    pub verbose: bool,
    /// Custom upstream provider URLs (override defaults).
    pub providers: ProviderUrls,
}

#[derive(Debug, Clone)]
pub struct ProviderUrls {
    pub anthropic: String,
    pub openai: String,
    pub google: String,
}

impl Default for ProviderUrls {
    fn default() -> Self {
        Self {
            anthropic: "https://api.anthropic.com".to_string(),
            openai: "https://api.openai.com".to_string(),
            google: "https://generativelanguage.googleapis.com".to_string(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            port: 8787,
            mode: DriveMode::Balanced,
            verbose: false,
            providers: ProviderUrls::default(),
        }
    }
}

impl ProxyConfig {
    /// Build a `ShiftConfig` for the optimization pipeline.
    pub fn shift_config(&self, provider: &str) -> ShiftConfig {
        ShiftConfig {
            mode: self.mode,
            svg_mode: SvgMode::Raster,
            provider: provider.to_string(),
            model: None,
            dry_run: false,
            verbose: self.verbose,
            profile_path: None,
            limits: shift_preflight::SafetyLimits::default(),
        }
    }
}

/// Shared proxy state, threaded through axum handlers via `State<ProxyState>`.
#[derive(Clone)]
pub struct ProxyState {
    pub config: ProxyConfig,
    pub http_client: reqwest::Client,
    pub session: Arc<SessionStats>,
}

impl ProxyState {
    pub fn new(config: ProxyConfig) -> Self {
        let http_client = reqwest::Client::builder()
            // Use connect_timeout, not total timeout — streaming responses
            // (SSE from Anthropic/OpenAI) can run for minutes.
            .connect_timeout(std::time::Duration::from_secs(30))
            // SECURITY: Do not follow redirects. Prevents SSRF via redirect
            // to internal services (e.g., cloud metadata endpoints).
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build HTTP client");

        Self {
            config,
            http_client,
            session: Arc::new(SessionStats::new()),
        }
    }
}

/// In-memory session statistics (mirrors TS SessionStats).
pub struct SessionStats {
    pub started_at: Instant,
    pub total_requests: AtomicU64,
    pub total_images: AtomicU64,
    pub total_images_modified: AtomicU64,
    pub total_bytes_saved: AtomicU64,
    pub token_savings: Mutex<TokenSavingsAccum>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TokenSavingsAccum {
    pub openai_before: u64,
    pub openai_after: u64,
    pub anthropic_before: u64,
    pub anthropic_after: u64,
}

impl Default for SessionStats {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStats {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            total_requests: AtomicU64::new(0),
            total_images: AtomicU64::new(0),
            total_images_modified: AtomicU64::new(0),
            total_bytes_saved: AtomicU64::new(0),
            token_savings: Mutex::new(TokenSavingsAccum::default()),
        }
    }

    /// Record stats from a completed optimization run.
    ///
    /// Atomics use `Ordering::Relaxed` because these are independent counters
    /// with no happens-before relationship — approximate consistency is fine
    /// for diagnostic stats.
    pub fn record(&self, report: &shift_preflight::Report) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_images
            .fetch_add(report.images_found as u64, Ordering::Relaxed);
        self.total_images_modified
            .fetch_add(report.images_modified as u64, Ordering::Relaxed);
        let saved = report.original_size.saturating_sub(report.transformed_size) as u64;
        self.total_bytes_saved.fetch_add(saved, Ordering::Relaxed);

        // Recover from mutex poisoning — the inner data (simple counters)
        // has no invariants that could be violated by a panic.
        let mut ts = self.token_savings.lock().unwrap_or_else(|e| e.into_inner());
        ts.openai_before += report.token_savings.openai_before;
        ts.openai_after += report.token_savings.openai_after;
        ts.anthropic_before += report.token_savings.anthropic_before;
        ts.anthropic_after += report.token_savings.anthropic_after;
    }

    /// Serialize to JSON for the /stats endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        let ts = self
            .token_savings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        serde_json::json!({
            "startedAt": format!("{:.0?}", self.started_at.elapsed()),
            "totalRequests": self.total_requests.load(Ordering::Relaxed),
            "totalImages": self.total_images.load(Ordering::Relaxed),
            "totalImagesModified": self.total_images_modified.load(Ordering::Relaxed),
            "totalBytesSaved": self.total_bytes_saved.load(Ordering::Relaxed),
            "tokenSavings": {
                "openai_before": ts.openai_before,
                "openai_after": ts.openai_after,
                "anthropic_before": ts.anthropic_before,
                "anthropic_after": ts.anthropic_after,
            }
        })
    }
}
