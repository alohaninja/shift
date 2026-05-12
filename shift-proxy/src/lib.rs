//! SHIFT native proxy — Rust HTTP server that intercepts AI API requests,
//! optimizes image payloads via `shift-preflight`, and forwards to upstream
//! providers. Replaces the Node.js/Hono proxy with a single-binary server.
//!
//! ## Architecture
//!
//! ```text
//! Client (OpenCode, Claude Code, Codex, etc.)
//!   │  (HTTP/1.1 or HTTP/2 over cleartext — h2c)
//!   ├── POST /v1/messages         → Anthropic (optimize + forward)
//!   ├── POST /messages            → Anthropic (rewrite → /v1/messages)
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
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower::Service;

pub use state::{ProxyConfig, ProxyState};

/// Build the axum router with all proxy routes.
pub fn create_app(config: ProxyConfig) -> Router {
    let state = ProxyState::new(config);
    routes::build_router(state)
}

/// Start the proxy server, blocking until shutdown signal.
///
/// The server auto-negotiates HTTP/1.1 and HTTP/2 over cleartext (h2c).
/// HTTP/2 provides multiplexed streams and header compression, which
/// improves SSE streaming performance for clients like OpenCode (Go)
/// that negotiate h2c by default.
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
        tracing::info!("shift proxy listening on http://{} (h1+h2c)", addr);
    }
    eprintln!("[shift] proxy listening on http://{}", addr);

    // Serve with auto HTTP/1.1 + HTTP/2 negotiation (h2c).
    // This replaces axum::serve which only supports HTTP/1.1.
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _remote_addr) = result?;
                let tower_service = app.clone();

                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let hyper_service = hyper::service::service_fn(move |req| {
                        let mut svc = tower_service.clone();
                        async move { svc.call(req).await }
                    });

                    // auto::Builder negotiates h2c with HTTP/1.1 fallback.
                    if let Err(err) = AutoBuilder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(io, hyper_service)
                        .await
                    {
                        // Connection errors are expected during shutdown
                        // or when clients disconnect early.
                        if !err.to_string().contains("connection closed") {
                            tracing::warn!("connection error: {}", err);
                        }
                    }
                });
            }
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received");
                break;
            }
        }
    }

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
