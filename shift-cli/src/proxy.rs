//! Proxy daemon lifecycle management.
//!
//! Provides `start`, `stop`, `status`, and `ensure` operations for the
//! SHIFT preflight proxy. The proxy is a native Rust HTTP server (axum)
//! running as a background daemon. No Node.js required.
//!
//! State is stored in `~/.shift/`:
//!   - `proxy.pid`  — PID of the running proxy process
//!   - `proxy.log`  — stdout/stderr from the proxy

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 8787;
const DEFAULT_MODE: &str = "balanced";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_SERVICE_ID: &str = "@shift-preflight/runtime proxy";

/// Return the `~/.shift/` directory, creating it if needed.
fn shift_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let dir = PathBuf::from(home).join(".shift");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("failed to create ~/.shift")?;
    }
    Ok(dir)
}

fn pid_file() -> Result<PathBuf> {
    Ok(shift_dir()?.join("proxy.pid"))
}

fn log_file() -> Result<PathBuf> {
    Ok(shift_dir()?.join("proxy.log"))
}

/// Read the stored PID, returning None if the file doesn't exist or is invalid.
fn read_pid() -> Option<u32> {
    let path = pid_file().ok()?;
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse().ok()
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    // kill(pid, 0) checks existence without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Probe the SHIFT proxy health endpoint.
/// Returns true only if the service identity matches (prevents port-squatting).
fn is_proxy_healthy(port: u16) -> bool {
    let url = format!("http://localhost:{}/health", port);
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(HEALTH_TIMEOUT))
            .build(),
    );
    let result = agent.get(&url).call();

    match result {
        Ok(response) => {
            if response.status().as_u16() != 200 {
                return false;
            }
            // Read body as string, then parse JSON
            let body_str: Result<String, _> = response.into_body().read_to_string();
            match body_str {
                Ok(s) => {
                    let json: Result<serde_json::Value, _> = serde_json::from_str(&s);
                    match json {
                        Ok(v) => v
                            .get("service")
                            .and_then(|v| v.as_str())
                            .map(|s| s == HEALTH_SERVICE_ID)
                            .unwrap_or(false),
                        Err(_) => false,
                    }
                }
                Err(_) => false,
            }
        }
        Err(_) => false,
    }
}

/// Wait for the proxy to become healthy, polling at intervals.
fn wait_for_healthy(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if is_proxy_healthy(port) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(HEALTH_POLL_INTERVAL);
    }
}

/// Start the proxy daemon. Idempotent — skips if already running.
pub fn start(port: Option<u16>, mode: Option<&str>, quiet: bool) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let mode = mode.unwrap_or(DEFAULT_MODE);

    // Already running?
    if is_proxy_healthy(port) {
        if !quiet {
            eprintln!("[shift] proxy already running on port {}", port);
        }
        return Ok(());
    }

    let log = log_file()?;
    let log_handle = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .context("failed to open proxy log file")?;
    let err_handle = log_handle
        .try_clone()
        .context("failed to clone log file handle")?;

    // Spawn ourselves with --foreground to run the native Rust proxy.
    // No Node.js / npx required — single binary, zero dependencies.
    let self_exe = std::env::current_exe().context("failed to determine shift-ai binary path")?;

    let child = Command::new(&self_exe)
        .args([
            "proxy",
            "start",
            "--foreground",
            "--port",
            &port.to_string(),
            "--mode",
            mode,
            "--quiet",
        ])
        .stdout(log_handle)
        .stderr(err_handle)
        .stdin(Stdio::null())
        .spawn()
        .context("failed to spawn proxy process")?;

    // Write PID file
    let pid = child.id();
    let pid_path = pid_file()?;
    fs::write(&pid_path, pid.to_string()).context("failed to write PID file")?;

    // Wait for healthy
    if wait_for_healthy(port, STARTUP_TIMEOUT) {
        if !quiet {
            eprintln!(
                "[shift] proxy started on port {} (pid {}, mode: {})",
                port, pid, mode
            );
        }
    } else {
        // Check if process died immediately
        if !is_pid_alive(pid) {
            // Clean up PID file
            let _ = fs::remove_file(&pid_path);
            anyhow::bail!(
                "proxy exited immediately — check {} for details",
                log.display()
            );
        }
        if !quiet {
            eprintln!(
                "[shift] proxy spawned (pid {}) but not yet responding on port {} — it may still be starting",
                pid, port
            );
        }
    }

    Ok(())
}

/// Stop the proxy daemon.
pub fn stop(quiet: bool) -> Result<()> {
    match read_pid() {
        Some(pid) if is_pid_alive(pid) => {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            // Wait briefly for process to exit
            let start = Instant::now();
            while is_pid_alive(pid) && start.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if is_pid_alive(pid) {
                // Force kill
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            let _ = fs::remove_file(pid_file()?);
            if !quiet {
                eprintln!("[shift] proxy stopped (pid {})", pid);
            }
        }
        Some(pid) => {
            // PID file exists but process is dead — clean up
            let _ = fs::remove_file(pid_file()?);
            if !quiet {
                eprintln!("[shift] proxy was not running (stale pid {})", pid);
            }
        }
        None => {
            if !quiet {
                eprintln!("[shift] proxy is not running (no PID file)");
            }
        }
    }
    Ok(())
}

/// Print proxy status.
pub fn status(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let healthy = is_proxy_healthy(port);
    let pid = read_pid();

    if healthy {
        if let Some(pid) = pid {
            println!("running (pid {}, port {})", pid, port);
        } else {
            println!("running (port {}, unknown pid)", port);
        }
    } else if let Some(pid) = pid {
        if is_pid_alive(pid) {
            println!("starting (pid {}, port {} not responding)", pid, port);
        } else {
            println!("stopped (stale pid {})", pid);
            let _ = fs::remove_file(pid_file()?);
        }
    } else {
        println!("stopped");
    }

    Ok(())
}

/// Ensure the proxy is running. Idempotent — starts if needed, no-op if healthy.
/// This is the primary command that all agent plugins should call.
pub fn ensure(port: Option<u16>, mode: Option<&str>, quiet: bool) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);

    if is_proxy_healthy(port) {
        return Ok(());
    }

    start(Some(port), mode, quiet)
}

/// Run the proxy server in the foreground (blocking).
/// Used by `shift-ai proxy start --foreground` and LaunchAgent/systemd.
pub fn run_foreground(port: u16, mode: &str) -> Result<()> {
    let drive_mode: shift_preflight::DriveMode =
        mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    let config = shift_proxy::ProxyConfig {
        port,
        mode: drive_mode,
        verbose: false,
        providers: shift_proxy::state::ProviderUrls::default(),
    };

    // Build a tokio runtime and run the proxy server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    rt.block_on(shift_proxy::start_server(config))
}
