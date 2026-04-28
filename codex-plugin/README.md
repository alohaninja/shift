# SHIFT — Codex CLI Integration

Transparent image optimization for OpenAI's Codex CLI. Automatically reduces
token costs and prevents oversized-image errors by routing API traffic through
the SHIFT preflight proxy.

## Quick Start (Recommended)

Run the interactive setup:

```bash
shift-ai setup
```

This detects Codex CLI and configures everything automatically.

## Manual Setup

### Option A: Shell Profile (Simplest)

Add to your `~/.zshrc` or `~/.bashrc`:

```bash
# Start SHIFT proxy if not running, set Codex base URL
eval "$(shift-ai env codex)"
```

Open a new terminal, then:

```bash
shift-ai proxy start
codex  # Now routes through SHIFT
```

### Option B: Per-Session Wrapper

Run Codex with the SHIFT proxy in a single command:

```bash
shift-ai proxy ensure --quiet && OPENAI_BASE_URL=http://localhost:8787 codex
```

Or create a shell alias:

```bash
alias codex-shift='shift-ai proxy ensure --quiet && OPENAI_BASE_URL=http://localhost:8787 codex'
```

### Option C: LaunchAgent (Always-On)

Install a macOS LaunchAgent so the proxy starts on login:

```bash
shift-ai setup
```

Then just set the env var in your shell profile:

```bash
eval "$(shift-ai env codex)"
```

## How It Works

```
Codex CLI
  → OPENAI_BASE_URL=http://localhost:8787
    → SHIFT Proxy (inspects + optimizes images)
      → https://api.openai.com/v1/chat/completions
```

1. Codex CLI sends API requests to `localhost:8787` instead of the OpenAI API
2. The SHIFT proxy inspects image payloads and optimizes oversized images
3. Optimized requests are forwarded to the real OpenAI API
4. Responses (including SSE streams) pass through unchanged

Auth headers pass through untouched — the proxy never stores credentials.

## Verification

```bash
shift-ai proxy status    # Check proxy is running
shift-ai gain            # View cumulative token savings
```

## Shared Proxy

The proxy on port 8787 is shared across all agents. If you also use Claude Code
or OpenCode, they can all use the same proxy instance:

```bash
shift-ai env --list   # Show env vars for all agents
```
