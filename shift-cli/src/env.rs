//! Agent environment variable generation.
//!
//! Each AI coding agent uses a different env var to override the API base URL.
//! This module outputs the correct shell commands for each agent so that
//! `eval "$(shift-ai env <agent>)"` in a shell profile is all that's needed.

use anyhow::{Context, Result};

const DEFAULT_PORT: u16 = 8787;

/// Agent-specific environment variable configuration.
struct AgentEnv {
    /// Human-readable agent name
    name: &'static str,
    /// Environment variable to set
    env_var: &'static str,
    /// Base URL value (with or without /v1 suffix depending on agent)
    url: String,
    /// Short description for comments
    description: &'static str,
}

fn agent_config(agent: &str, port: u16) -> Result<AgentEnv> {
    match agent {
        "opencode" => Ok(AgentEnv {
            name: "OpenCode",
            env_var: "ANTHROPIC_BASE_URL",
            // OpenCode requires /v1 suffix — without it, requests go to /messages instead of /v1/messages
            url: format!("http://localhost:{}/v1", port),
            description: "OpenCode (requires /v1 suffix)",
        }),
        "claude" | "claude-code" => Ok(AgentEnv {
            name: "Claude Code",
            env_var: "ANTHROPIC_BASE_URL",
            url: format!("http://localhost:{}", port),
            description: "Claude Code",
        }),
        "codex" | "openai" => Ok(AgentEnv {
            name: "Codex CLI / OpenAI",
            env_var: "OPENAI_BASE_URL",
            url: format!("http://localhost:{}", port),
            description: "Codex CLI / OpenAI agents",
        }),
        "gemini" => Ok(AgentEnv {
            name: "Gemini CLI",
            env_var: "GEMINI_API_BASE",
            url: format!("http://localhost:{}", port),
            description: "Gemini CLI",
        }),
        "cursor" => Ok(AgentEnv {
            name: "Cursor",
            env_var: "OPENAI_BASE_URL",
            url: format!("http://localhost:{}", port),
            description: "Cursor AI",
        }),
        _ => anyhow::bail!(
            "unknown agent '{}'. Supported: opencode, claude-code, codex, gemini, cursor",
            agent
        ),
    }
}

/// Print the env export command for a single agent.
pub fn print_env(agent: &str, port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let config = agent_config(agent, port)?;

    println!("# SHIFT proxy — {} ({})", config.description, config.name);
    println!("export {}=\"{}\"", config.env_var, config.url);

    Ok(())
}

/// Print env export commands for all supported agents.
pub fn print_env_all(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let agents = ["opencode", "claude-code", "codex", "gemini", "cursor"];

    println!("# SHIFT proxy — environment variables for all supported agents");
    println!("# Add to your shell profile (~/.zshrc, ~/.bashrc, etc.):");
    println!("#   eval \"$(shift-ai env --all)\"");
    println!();

    for agent in &agents {
        let config = agent_config(agent, port)?;
        println!("# {}", config.description);
        println!("export {}=\"{}\"", config.env_var, config.url);
        println!();
    }

    Ok(())
}

/// Print a table of all supported agents and their env vars (for --list).
pub fn print_agent_list(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let agents = [
        ("opencode", "OpenCode"),
        ("claude-code", "Claude Code"),
        ("codex", "Codex CLI"),
        ("gemini", "Gemini CLI"),
        ("cursor", "Cursor"),
    ];

    println!("{:<15} {:<25} {}", "Agent", "Env Var", "Base URL");
    println!("{}", "─".repeat(70));

    for (key, name) in &agents {
        let config = agent_config(key, port).context("agent config failed")?;
        println!("{:<15} {:<25} {}", name, config.env_var, config.url);
    }

    println!();
    println!("Usage: eval \"$(shift-ai env <agent>)\"");

    Ok(())
}
