//! Agent configuration output.
//!
//! Each AI coding agent uses a different mechanism to override the API base URL.
//! This module outputs the correct configuration for each agent:
//!   - Shell env vars (Claude Code, OpenCode)
//!   - TOML config snippets (Codex CLI)
//!   - Manual instructions (Cursor)

use anyhow::Result;

const DEFAULT_PORT: u16 = 8787;

/// Print configuration for a single agent.
pub fn print_env(agent: &str, port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);

    match agent {
        "opencode" => {
            println!("# SHIFT proxy — OpenCode");
            println!("# OpenCode requires the /v1 suffix in the base URL.");
            println!("# Recommended: use the @shift-preflight/opencode-plugin instead.");
            println!("# If configuring manually, add to ~/.config/opencode/opencode.json:");
            println!("#   \"provider\": {{ \"anthropic\": {{ \"options\": {{ \"baseURL\": \"http://localhost:{}/v1\" }} }} }}", port);
        }
        "claude" | "claude-code" => {
            println!("# SHIFT proxy — Claude Code");
            println!("# Add to your shell profile (~/.zshrc, ~/.bashrc):");
            println!("export ANTHROPIC_BASE_URL=\"http://localhost:{}\"", port);
            println!();
            println!("# Or add to ~/.claude/settings.json:");
            println!(
                "# {{ \"env\": {{ \"ANTHROPIC_BASE_URL\": \"http://localhost:{}\" }} }}",
                port
            );
        }
        "codex" => {
            println!("# SHIFT proxy — Codex CLI");
            println!("# Codex CLI uses ~/.codex/config.toml (NOT environment variables).");
            println!("# Add the following line to ~/.codex/config.toml:");
            println!();
            println!("openai_base_url = \"http://localhost:{}\"", port);
            println!();
            println!("# Or pass as a one-off flag:");
            println!("# codex -c 'openai_base_url=\"http://localhost:{}\"'", port);
        }
        "cursor" => {
            println!("# SHIFT proxy — Cursor");
            println!("# Cursor requires manual configuration through its settings UI.");
            println!("# 1. Open Cursor Settings > Models");
            println!("# 2. Enter your own OpenAI API key");
            println!("# 3. Set \"Override OpenAI Base URL\" to:");
            println!("#    http://localhost:{}/v1", port);
            println!(
                "# Note: This only works with your own API key, not Cursor's built-in models."
            );
        }
        _ => anyhow::bail!(
            "unknown agent '{}'. Supported: opencode, claude-code, codex, cursor",
            agent
        ),
    }

    Ok(())
}

/// Print configuration for all supported agents.
pub fn print_env_all(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);

    println!("# SHIFT proxy — configuration for all supported agents");
    println!("# Only Claude Code uses shell env vars. Other agents need");
    println!("# config files or UI settings. Run `shift-ai setup` for");
    println!("# automatic configuration of all detected agents.");
    println!();
    println!("# Claude Code (env var — add to shell profile)");
    println!("export ANTHROPIC_BASE_URL=\"http://localhost:{}\"", port);
    println!();
    println!("# Codex CLI — add to ~/.codex/config.toml:");
    println!("# openai_base_url = \"http://localhost:{}\"", port);
    println!();
    println!("# OpenCode — use @shift-preflight/opencode-plugin (recommended)");
    println!("# Cursor — set in Settings > Models > Override OpenAI Base URL");

    Ok(())
}

/// Print a table of all supported agents and their configuration methods.
pub fn print_agent_list(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);

    let base = format!("http://localhost:{}", port);
    let base_v1 = format!("http://localhost:{}/v1", port);
    let codex_val = format!("openai_base_url = \"{}\"", base);

    println!("Agent           Method                    Configuration");
    let sep = "─".repeat(75);
    println!("{sep}");
    println!(
        "{:<16}{:<26}{base}",
        "Claude Code", "ANTHROPIC_BASE_URL env"
    );
    println!(
        "{:<16}{:<26}{codex_val}",
        "Codex CLI", "~/.codex/config.toml"
    );
    println!(
        "{:<16}{:<26}{base_v1}",
        "OpenCode", "opencode-plugin (auto)"
    );
    println!("{:<16}{:<26}{base_v1}", "Cursor", "Settings UI (manual)");
    println!();
    println!("Claude Code:  eval \"$(shift-ai env claude-code)\"");
    println!("Codex CLI:    shift-ai env codex  (prints TOML snippet)");
    println!("All agents:   shift-ai setup      (interactive, recommended)");

    Ok(())
}
