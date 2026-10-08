//! Interactive setup for multi-agent SHIFT integration.
//!
//! Detects which AI coding agents are installed and configures each one
//! to route API traffic through the SHIFT proxy. Also optionally installs
//! a macOS LaunchAgent for auto-start on login.
//!
//! Supported agents and their configuration mechanisms:
//!   - OpenCode:    writes opencode.json (plugin + provider.baseURL)
//!   - Claude Code: writes ~/.claude/settings.json (env.ANTHROPIC_BASE_URL)
//!   - Codex CLI:   writes ~/.codex/config.toml (openai_base_url)
//!   - Cursor:      prints manual instructions (UI-only setting)

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_PORT: u16 = 8787;
#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "com.shift-ai.proxy";

/// Detected agent information.
struct DetectedAgent {
    name: &'static str,
    key: &'static str,
    detected: bool,
    configured: bool,
}

/// Check if a command exists on PATH.
fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a directory exists.
fn dir_exists(path: &str) -> bool {
    let expanded = shellexpand(path);
    std::path::Path::new(&expanded).exists()
}

/// Simple ~ expansion.
fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

/// Detect installed agents.
fn detect_agents() -> Vec<DetectedAgent> {
    vec![
        DetectedAgent {
            name: "OpenCode",
            key: "opencode",
            detected: command_exists("opencode") || dir_exists("~/.config/opencode"),
            configured: false,
        },
        DetectedAgent {
            name: "Claude Code",
            key: "claude-code",
            detected: command_exists("claude") || dir_exists("~/.claude"),
            configured: false,
        },
        DetectedAgent {
            name: "Codex CLI",
            key: "codex",
            detected: command_exists("codex") || dir_exists("~/.codex"),
            configured: false,
        },
        DetectedAgent {
            name: "Cursor",
            key: "cursor",
            detected: command_exists("cursor") || dir_exists("~/.cursor"),
            configured: false,
        },
    ]
}

/// Check prerequisites.
fn check_prerequisites() -> Result<(bool, bool)> {
    let shift_ok = command_exists("shift-ai");
    let npx_ok = command_exists("npx");
    Ok((shift_ok, npx_ok))
}

// ── macOS LaunchAgent ────────────────────────────────────────────────

/// Generate the macOS LaunchAgent plist content.
#[cfg(target_os = "macos")]
fn launchagent_plist(shift_ai_path: &str, port: u16) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>proxy</string>
        <string>start</string>
        <string>--port</string>
        <string>{port}</string>
        <string>--foreground</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{home}/.shift/proxy.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/.shift/proxy.log</string>
</dict>
</plist>"#,
        label = LAUNCH_AGENT_LABEL,
        binary = shift_ai_path,
        port = port,
        home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()),
    )
}

/// Install the macOS LaunchAgent.
#[cfg(target_os = "macos")]
fn install_launchagent(port: u16) -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let agents_dir = PathBuf::from(&home).join("Library/LaunchAgents");
    fs::create_dir_all(&agents_dir).context("failed to create LaunchAgents directory")?;

    let plist_path = agents_dir.join(format!("{}.plist", LAUNCH_AGENT_LABEL));

    // Find shift-ai binary path
    let shift_ai_path = Command::new("which")
        .arg("shift-ai")
        .output()
        .context("failed to find shift-ai")?;
    let binary = String::from_utf8_lossy(&shift_ai_path.stdout)
        .trim()
        .to_string();
    if binary.is_empty() {
        anyhow::bail!("shift-ai not found on PATH");
    }

    let content = launchagent_plist(&binary, port);
    fs::write(&plist_path, &content).context("failed to write LaunchAgent plist")?;

    // Unload if already loaded (ignore errors)
    let _ = Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .output();

    // Load
    Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .status()
        .context("failed to load LaunchAgent")?;

    Ok(())
}

// ── Per-agent configuration ──────────────────────────────────────────

/// Configure Claude Code by writing/updating ~/.claude/settings.json.
fn configure_claude_code(port: u16) -> Result<bool> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let settings_dir = PathBuf::from(&home).join(".claude");
    fs::create_dir_all(&settings_dir)?;

    let settings_path = settings_dir.join("settings.json");

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Add env.ANTHROPIC_BASE_URL
    let env = settings
        .as_object_mut()
        .context("settings is not an object")?
        .entry("env")
        .or_insert_with(|| serde_json::json!({}));

    if let Some(env_obj) = env.as_object_mut() {
        let url = format!("http://localhost:{}", port);
        env_obj.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            serde_json::Value::String(url),
        );
    }

    let content = serde_json::to_string_pretty(&settings)?;
    fs::write(&settings_path, content)?;
    Ok(true)
}

/// Configure OpenCode by updating opencode.json at a specific path.
/// Returns `Ok(true)` if configuration was written, `Ok(false)` if the
/// config file does not exist.
fn configure_opencode_at(config_path: &Path, port: u16) -> Result<bool> {
    use std::io::Write;

    if !config_path.exists() {
        return Ok(false);
    }

    // Replace the target atomically while preserving user-managed config symlinks.
    let config_path = fs::canonicalize(config_path)?;
    let permissions = fs::metadata(&config_path)?.permissions();
    let content = fs::read_to_string(&config_path)?;
    let mut config: serde_json::Value = serde_json::from_str(&content)
        .context("cannot parse opencode.json; configuration left unchanged")?;

    // Respect native V2 keys when present; supported V1 config stays in V1 form.
    let provider_key = if config["providers"].get("anthropic").is_some() {
        "providers"
    } else if config["provider"].get("anthropic").is_some() {
        "provider"
    } else if config.get("providers").is_some() {
        "providers"
    } else {
        "provider"
    };
    let options_key = if provider_key == "providers" {
        "settings"
    } else {
        "options"
    };
    let plugin_key = if config.get("plugins").is_some() {
        "plugins"
    } else {
        "plugin"
    };
    let provider = config
        .as_object_mut()
        .context("config is not an object")?
        .entry(provider_key)
        .or_insert_with(|| serde_json::json!({}));
    let anthropic = provider
        .as_object_mut()
        .context("provider is not an object")?
        .entry("anthropic")
        .or_insert_with(|| serde_json::json!({}));
    let options = anthropic
        .as_object_mut()
        .context("anthropic is not an object")?
        .entry(options_key)
        .or_insert_with(|| serde_json::json!({}));

    if let Some(opts) = options.as_object_mut() {
        // OpenCode's Anthropic client uses baseURL as the full prefix and
        // appends only `/messages` (NOT `/v1/messages`).  Therefore we must
        // include `/v1` here so the final URL becomes `…/v1/messages`.
        let url = format!("http://localhost:{}/v1", port);
        opts.insert("baseURL".to_string(), serde_json::Value::String(url));
    }

    // Add plugin if not present
    let plugins = config
        .as_object_mut()
        .unwrap()
        .entry(plugin_key)
        .or_insert_with(|| serde_json::json!([]));
    if let Some(arr) = plugins.as_array_mut() {
        let plugin_name = "@shift-preflight/opencode-plugin";
        if !arr.iter().any(|v| {
            v.as_str() == Some(plugin_name)
                || v.get("package").and_then(|p| p.as_str()) == Some(plugin_name)
        }) {
            arr.push(serde_json::Value::String(plugin_name.to_string()));
        }
    }

    let output = serde_json::to_string_pretty(&config)?;
    let mut replacement = tempfile::Builder::new()
        .prefix(".opencode-")
        .tempfile_in(config_path.parent().context("config path has no parent")?)?;
    replacement.write_all(output.as_bytes())?;
    replacement.as_file().set_permissions(permissions)?;
    replacement.as_file().sync_all()?;
    replacement
        .persist(&config_path)
        .context("failed to replace opencode.json")?;
    Ok(true)
}

/// Configure OpenCode by updating ~/.config/opencode/opencode.json.
fn configure_opencode(port: u16) -> Result<bool> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let config_path = PathBuf::from(&home).join(".config/opencode/opencode.json");
    configure_opencode_at(&config_path, port)
}

/// Configure Codex CLI by writing/updating ~/.codex/config.toml.
fn configure_codex(port: u16) -> Result<bool> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let config_dir = PathBuf::from(&home).join(".codex");
    fs::create_dir_all(&config_dir)?;

    let config_path = config_dir.join("config.toml");
    let key = "openai_base_url";
    let value = format!("http://localhost:{}", port);

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        // Check if already set
        if content.contains(key) {
            // Replace existing line
            let mut new_lines = Vec::new();
            for line in content.lines() {
                if line.trim_start().starts_with(key) {
                    new_lines.push(format!("{} = \"{}\"", key, value));
                } else {
                    new_lines.push(line.to_string());
                }
            }
            fs::write(&config_path, new_lines.join("\n") + "\n")?;
        } else {
            // Append
            let mut content = content;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&format!("{} = \"{}\"\n", key, value));
            fs::write(&config_path, content)?;
        }
    } else {
        // Create new config
        fs::write(&config_path, format!("{} = \"{}\"\n", key, value))?;
    }

    Ok(true)
}

// ── Stale OpenCode plugin cache detection ────────────────────────────

/// Compare two semver strings. Returns true if `cached` is older than `current`.
fn is_older_semver(cached: &str, current: &str) -> bool {
    let parse = |s: &str| -> Option<(u64, u64, u64)> {
        let parts: Vec<&str> = s.trim().trim_start_matches('v').split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    };

    match (parse(cached), parse(current)) {
        (Some(c), Some(cur)) => c < cur,
        _ => false,
    }
}

/// Check for a stale OpenCode plugin cache at a specific directory.
///
/// Supports the V1 package directory and V2 timestamped generation directories.
/// Only stale generations are deleted; current generations remain in place.
///
/// Returns `Some(old_version)` if a stale cache was cleared, `None` otherwise.
pub(crate) fn check_and_clear_stale_opencode_cache_at(cache_dir: &Path) -> Result<Option<String>> {
    if !cache_dir.exists() || !fs::symlink_metadata(cache_dir)?.is_dir() {
        return Ok(None);
    }
    if cache_dir.join("node_modules").exists() {
        return clear_stale_opencode_generation(cache_dir);
    }
    let mut cleared = None;
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if !entry.file_type()?.is_dir()
            || !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        if let Some(version) = clear_stale_opencode_generation(&entry.path())? {
            cleared.get_or_insert(version);
        }
    }
    Ok(cleared)
}

fn clear_stale_opencode_generation(cache_dir: &Path) -> Result<Option<String>> {
    let pkg_json = cache_dir.join("node_modules/@shift-preflight/opencode-plugin/package.json");

    if !pkg_json.exists() {
        return Ok(None);
    }

    let content =
        fs::read_to_string(&pkg_json).context("failed to read cached plugin package.json")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&content).context("failed to parse cached plugin package.json")?;

    let cached_version = match parsed.get("version").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return Ok(None),
    };

    let current_version = env!("CARGO_PKG_VERSION");

    if is_older_semver(&cached_version, current_version) {
        // Don't follow symlinks — only delete real directories
        let meta = fs::symlink_metadata(cache_dir)?;
        if !meta.is_dir() {
            return Ok(None);
        }
        fs::remove_dir_all(cache_dir).context("failed to remove stale OpenCode plugin cache")?;
        Ok(Some(cached_version))
    } else {
        Ok(None)
    }
}

/// Check for a stale OpenCode plugin cache at the default location.
///
/// Returns `Some(old_version)` if a stale cache was cleared, `None` otherwise.
pub(crate) fn check_and_clear_stale_opencode_cache() -> Result<Option<String>> {
    let cache_root = match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(".cache"),
            None => return Ok(None),
        },
    };
    let mut cleared = None;
    for layout in ["packages", "npm"] {
        let cache_dir = cache_root
            .join("opencode")
            .join(layout)
            .join("@shift-preflight/opencode-plugin@latest");
        if let Some(version) = check_and_clear_stale_opencode_cache_at(&cache_dir)? {
            cleared.get_or_insert(version);
        }
    }
    Ok(cleared)
}

// ── Interactive setup ────────────────────────────────────────────────

/// Run the interactive setup.
pub fn run_setup() -> Result<()> {
    use colored::Colorize;
    use std::io::{self, IsTerminal, Write};

    let use_color = io::stdout().is_terminal();
    let port = DEFAULT_PORT;

    // Header
    if use_color {
        println!("{}", "SHIFT Setup".bold().green());
        println!("{}", "═".repeat(50).green());
    } else {
        println!("=== SHIFT Setup ===");
    }
    println!();

    // Prerequisites
    println!("Checking prerequisites...");
    let (shift_ok, npx_ok) = check_prerequisites()?;
    if shift_ok {
        println!(
            "  {} shift-ai v{} installed",
            "✓".green(),
            env!("CARGO_PKG_VERSION")
        );
    } else {
        println!("  {} shift-ai not found", "✗".red());
        println!();
        println!("Install shift-ai first:");
        println!("  brew install alohaninja/shift/shift-ai");
        println!("  # or: cargo install shift-preflight-cli");
        return Ok(());
    }
    if npx_ok {
        println!("  {} npx available", "✓".green());
    } else {
        println!("  {} npx not found (required for proxy)", "✗".red());
        println!();
        println!("Install Node.js: brew install node");
        return Ok(());
    }
    println!();

    // Detect agents
    println!("Detecting AI agents...");
    let mut agents = detect_agents();
    let any_detected = agents.iter().any(|a| a.detected);

    for agent in &agents {
        if agent.detected {
            println!("  {} {} — found", "✓".green(), agent.name);
        } else {
            println!("  {} {} — not found", "✗".red().dimmed(), agent.name);
        }
    }
    println!();

    if !any_detected {
        println!("No AI agents detected. You can still start the proxy manually:");
        println!("  shift-ai proxy start");
        println!();
        println!("Then configure your agent:");
        println!("  shift-ai env --list");
        return Ok(());
    }

    // Ask for confirmation
    print!("Configure detected agents and install LaunchAgent? [Y/n] ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    if input == "n" || input == "no" {
        println!("Setup cancelled.");
        return Ok(());
    }
    println!();

    // Configure each detected agent
    println!("Installing...");

    for agent in agents.iter_mut() {
        if !agent.detected {
            continue;
        }

        let result = match agent.key {
            "opencode" => configure_opencode(port),
            "claude-code" => configure_claude_code(port),
            "codex" => configure_codex(port),
            "cursor" => Ok(false), // UI-only, handled below
            _ => Ok(false),
        };

        match result {
            Ok(true) => {
                agent.configured = true;
                println!("  {} {} configured", "✓".green(), agent.name);
            }
            Ok(false) if agent.key == "cursor" => {
                println!(
                    "  {} {} — open Settings > Models > Override OpenAI Base URL:",
                    "→".yellow(),
                    agent.name,
                );
                println!("         http://localhost:{}/v1", port);
            }
            Ok(false) => {
                println!(
                    "  {} {} — run: shift-ai env {}",
                    "→".yellow(),
                    agent.name,
                    agent.key
                );
            }
            Err(e) => {
                println!("  {} {} — failed: {}", "✗".red(), agent.name, e);
            }
        }

        // Always check for stale OpenCode plugin cache, regardless of config result
        if agent.key == "opencode" {
            match check_and_clear_stale_opencode_cache() {
                Ok(Some(old_ver)) => {
                    println!(
                        "  {} Cleared stale OpenCode plugin cache (was v{}, latest v{})",
                        "✓".green(),
                        old_ver,
                        env!("CARGO_PKG_VERSION"),
                    );
                }
                Ok(None) => {} // cache current or absent, nothing to do
                Err(e) => {
                    println!(
                        "  {} OpenCode plugin cache check failed: {}",
                        "→".yellow(),
                        e,
                    );
                }
            }
        }
    }

    // Install LaunchAgent (macOS only)
    #[cfg(target_os = "macos")]
    {
        match install_launchagent(port) {
            Ok(()) => {
                println!(
                    "  {} LaunchAgent installed (auto-start on login)",
                    "✓".green()
                );
            }
            Err(e) => {
                println!("  {} LaunchAgent — failed: {}", "✗".red(), e);
            }
        }
    }

    // Start proxy now
    match crate::proxy::ensure(Some(port), None, false) {
        Ok(()) => {
            println!("  {} Proxy healthy on port {}", "✓".green(), port);
        }
        Err(e) => {
            println!("  {} Proxy — failed to start: {}", "✗".red(), e);
        }
    }

    println!();
    if use_color {
        println!("{}", "Done!".bold().green());
    } else {
        println!("Done!");
    }
    println!("All API traffic now flows through SHIFT.");
    println!("Run `shift-ai gain` to see cumulative token savings.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a fake OpenCode plugin cache directory with a package.json
    /// containing the given version string.
    fn make_fake_cache(base: &Path, version: &str) {
        let pkg_dir = base.join("node_modules/@shift-preflight/opencode-plugin");
        fs::create_dir_all(&pkg_dir).unwrap();
        let pkg_json = pkg_dir.join("package.json");
        let content = serde_json::json!({
            "name": "@shift-preflight/opencode-plugin",
            "version": version
        });
        fs::write(&pkg_json, serde_json::to_string_pretty(&content).unwrap()).unwrap();
    }

    #[test]
    fn test_check_stale_cache_v2_generations() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp
            .path()
            .join("npm/@shift-preflight/opencode-plugin@latest");
        let stale = cache.join("1790261017722");
        let stale2 = cache.join("1790261017723");
        let current = cache.join("1790261017724");
        let unrelated = cache.join("unrelated");
        let pinned = tmp
            .path()
            .join("npm/@shift-preflight/opencode-plugin@0.0.1/1790261017722");
        make_fake_cache(&stale, "0.0.1");
        make_fake_cache(&stale2, "0.0.1");
        make_fake_cache(&current, env!("CARGO_PKG_VERSION"));
        make_fake_cache(&unrelated, "0.0.1");
        make_fake_cache(&pinned, "0.0.1");

        assert_eq!(
            check_and_clear_stale_opencode_cache_at(&cache).unwrap(),
            Some("0.0.1".into())
        );
        assert!(!stale.exists());
        assert!(!stale2.exists());
        assert!(current.exists());
        assert!(unrelated.exists());
        assert!(pinned.exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_stale_cache_skips_symlinked_generations() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&cache).unwrap();
        make_fake_cache(&outside, "0.0.1");
        std::os::unix::fs::symlink(&outside, cache.join("1790261017722")).unwrap();
        assert_eq!(
            check_and_clear_stale_opencode_cache_at(&cache).unwrap(),
            None
        );
        assert!(outside.exists());
    }

    #[test]
    fn test_check_stale_cache_detects_old_version() {
        let tmp = tempfile::tempdir().unwrap();
        // Take ownership so TempDir::drop won't try to delete after remove_dir_all
        let tmp_path = tmp.keep();
        let cache_dir = tmp_path.clone();

        // The CLI version from Cargo.toml (e.g. "0.9.4").
        // Create a cache with a version guaranteed to be older.
        make_fake_cache(&cache_dir, "0.0.1");

        let result = check_and_clear_stale_opencode_cache_at(&cache_dir).unwrap();
        assert_eq!(result, Some("0.0.1".to_string()));
        // Directory should have been deleted
        assert!(!cache_dir.exists());
        // Clean up the parent if it still exists (it shouldn't, since cache_dir == tmp_path)
        let _ = fs::remove_dir_all(&tmp_path);
    }

    #[test]
    fn test_check_stale_cache_skips_current_version() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().to_path_buf();

        let current = env!("CARGO_PKG_VERSION");
        make_fake_cache(&cache_dir, current);

        let result = check_and_clear_stale_opencode_cache_at(&cache_dir).unwrap();
        assert_eq!(result, None);
        // Directory should still exist
        assert!(cache_dir.exists());
    }

    #[test]
    fn test_check_stale_cache_skips_missing_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("nonexistent");
        // Don't create anything — cache_dir does not exist

        let result = check_and_clear_stale_opencode_cache_at(&cache_dir).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    #[cfg(unix)]
    fn test_configure_opencode_failed_replacement_preserves_file() {
        use std::os::unix::fs::PermissionsExt;
        // Root can write through directory permissions; this probe needs a normal user.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("opencode.json");
        let original = "{\"plugins\":[\"preserve-me\"]}";
        fs::write(&path, original).unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = configure_opencode_at(&path, 8787);
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    #[cfg(unix)]
    fn test_configure_opencode_preserves_symlink_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real-config.json");
        let link = tmp.path().join("opencode.json");
        fs::write(&target, "{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        configure_opencode_at(&link, 8787).unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(
            config["provider"]["anthropic"]["options"]["baseURL"],
            "http://localhost:8787/v1"
        );
    }

    #[test]
    fn test_configure_opencode_preserves_legacy_anthropic_with_other_native_providers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("opencode.json");
        let initial = serde_json::json!({
            "providers": {"openai": {}},
            "provider": {"anthropic": {
                "name": "My Claude",
                "options": {"timeout": 120000, "headers": {"x-review": "preserve"}},
                "models": {"custom-claude": {"name": "Custom Claude", "id": "claude-sonnet-4-5"}}
            }}
        });
        fs::write(&path, initial.to_string()).unwrap();
        configure_opencode_at(&path, 8787).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(config["providers"].get("anthropic").is_none());
        let mut expected = initial["provider"]["anthropic"].clone();
        expected["options"]["baseURL"] = "http://localhost:8787/v1".into();
        assert_eq!(config["provider"]["anthropic"], expected);
        assert_eq!(config["providers"], initial["providers"]);
    }

    #[test]
    fn test_configure_opencode_parse_error_preserves_file() {
        for original in [
            "{\"plugins\": [\"my-plugin\"], // keep this comment\n}",
            "{\"providers\": {\"anthropic\": ",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("opencode.json");
            fs::write(&path, original).unwrap();

            assert!(configure_opencode_at(&path, 8787).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn test_configure_opencode_preserves_native_v2_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("opencode.json");
        let initial = serde_json::json!({
            "plugins": [{"package": "another-plugin", "options": {"enabled": true}}],
            "providers": {"anthropic": {"settings": {"timeout": 120000}}}
        });
        fs::write(&path, initial.to_string()).unwrap();
        configure_opencode_at(&path, 8787).unwrap();
        configure_opencode_at(&path, 8787).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            config["providers"]["anthropic"]["settings"]["baseURL"],
            "http://localhost:8787/v1"
        );
        assert_eq!(
            config["providers"]["anthropic"]["settings"]["timeout"],
            120000
        );
        assert_eq!(config["plugins"][0], initial["plugins"][0]);
        assert_eq!(config["plugins"].as_array().unwrap().len(), 2);
        assert_eq!(config["plugins"][1], "@shift-preflight/opencode-plugin");
        assert!(config.get("plugin").is_none());
        assert!(config.get("provider").is_none());
    }

    #[test]
    fn test_configure_opencode_sets_correct_baseurl() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("opencode.json");

        // Write a minimal config
        let initial = serde_json::json!({
            "$schema": "https://opencode.ai/config.json"
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        let result = configure_opencode_at(&config_path, 8787).unwrap();
        assert!(result);

        // Read back and verify
        let content = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();

        let base_url = config["provider"]["anthropic"]["options"]["baseURL"]
            .as_str()
            .unwrap();
        assert_eq!(base_url, "http://localhost:8787/v1");
        // OpenCode appends only `/messages`, so baseURL MUST include `/v1`.
        assert!(
            base_url.ends_with("/v1"),
            "OpenCode baseURL must include /v1, got: {}",
            base_url
        );
        assert!(
            !base_url.ends_with("/v1/v1"),
            "must not have double /v1, got: {}",
            base_url
        );

        // Verify plugin was added
        let plugins = config["plugin"].as_array().unwrap();
        assert!(plugins
            .iter()
            .any(|v| v.as_str() == Some("@shift-preflight/opencode-plugin")));
    }

    #[test]
    fn test_is_older_semver() {
        assert!(is_older_semver("0.0.1", "0.9.4"));
        assert!(is_older_semver("0.9.3", "0.9.4"));
        assert!(is_older_semver("0.8.0", "1.0.0"));
        assert!(!is_older_semver("0.9.4", "0.9.4")); // same
        assert!(!is_older_semver("1.0.0", "0.9.4")); // newer
        assert!(!is_older_semver("0.9.5", "0.9.4")); // newer patch
        assert!(!is_older_semver("invalid", "0.9.4")); // invalid
    }

    /// Pre-release versions (e.g. "0.9.4-beta.1") contain a hyphen in the
    /// patch component, so `parse()` returns `None` and `is_older_semver`
    /// safely returns `false` — meaning we do NOT delete the cache.  This
    /// is the conservative choice: don't delete what we can't understand.
    #[test]
    fn test_is_older_semver_prerelease() {
        assert!(!is_older_semver("0.9.4-beta.1", "0.9.4"));
        assert!(!is_older_semver("0.9.4", "0.9.5-rc.1"));
        assert!(!is_older_semver("1.0.0-alpha", "1.0.0"));
    }

    /// Validates the core migration scenario: an existing opencode.json with
    /// the OLD baseURL (missing `/v1`) gets corrected to include `/v1`.
    #[test]
    fn test_configure_opencode_overwrites_old_baseurl() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("opencode.json");

        // Write a config with the OLD wrong baseURL (no /v1 suffix)
        let initial = serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "provider": {
                "anthropic": {
                    "options": {
                        "baseURL": "http://localhost:8787"
                    }
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        let result = configure_opencode_at(&config_path, 8787).unwrap();
        assert!(result);

        // Read back and verify the baseURL was corrected
        let content = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();

        let base_url = config["provider"]["anthropic"]["options"]["baseURL"]
            .as_str()
            .unwrap();
        assert_eq!(
            base_url, "http://localhost:8787/v1",
            "Old baseURL without /v1 must be overwritten to include /v1"
        );
    }
}
