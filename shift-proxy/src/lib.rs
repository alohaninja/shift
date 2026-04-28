//! SHIFT native proxy — Rust HTTP server that intercepts AI API requests,
//! optimizes image payloads via `shift-preflight`, and forwards to upstream
//! providers. Replaces the Node.js/Hono proxy with a single-binary server.
//!
//! ## Architecture
//!
//! ```text
//! Client (OpenCode, Claude Code, Codex, etc.)
//!   │
//!   ├── POST /v1/messages         → Anthropic (optimize + forward)
//!   ├── POST /v1/chat/completions → OpenAI   (optimize + forward)
//!   ├── POST /v1beta/models/*     → Google   (passthrough)
//!   ├── GET  /health              → Status
//!   ├── GET  /stats               → Session stats
//!   └── POST /*                   → Auto-detect provider (passthrough)
//! ```

pub mod forward;
pub mod optimize;
pub mod routes;
pub mod state;

use axum::Router;
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub use state::{ProxyConfig, ProxyState};

/// Build the axum router with all proxy routes.
pub fn create_app(config: ProxyConfig) -> Router {
    let state = ProxyState::new(config);
    routes::build_router(state)
}

/// Start the proxy server, blocking until shutdown signal.
pub async fn start_server(config: ProxyConfig) -> anyhow::Result<()> {
    // Initialize tracing subscriber so that tracing::warn!/error!/info!
    // calls in route handlers are actually visible on stderr.
    let filter = if config.verbose {
        "shift_proxy=debug,tower_http=debug"
    } else {
        "shift_proxy=warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_target(false)
        .init();

    let port = config.port;
    let verbose = config.verbose;
    let app = create_app(config);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;

    if verbose {
        tracing::info!("shift proxy listening on http://{}", addr);
    }
    eprintln!("[shift] proxy listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
