# SHIFT — Codex CLI Integration

Transparent image optimization for OpenAI's Codex CLI. Automatically reduces
token costs and prevents oversized-image errors by routing API traffic through
the SHIFT preflight proxy.

## Quick Start (Recommended)

Run the interactive setup:

```bash
shift-ai setup
```

This detects Codex CLI and writes `openai_base_url` to `~/.codex/config.toml`
automatically.

## Manual Setup

### Option A: config.toml (Recommended)

Add to `~/.codex/config.toml`:

```toml
openai_base_url = "http://localhost:8787"
```

Then start the proxy:

```bash
shift-ai proxy start
```

> **Note:** Codex CLI uses its own TOML config — it does **not** read the
> `OPENAI_BASE_URL` environment variable.

### Option B: CLI Flag (Per-Session)

```bash
shift-ai proxy ensure --quiet
codex -c 'openai_base_url="http://localhost:8787"'
```

### Option C: Custom Model Provider

For more control, define a custom provider in `~/.codex/config.toml`:

```toml
model = "gpt-4.1"
model_provider = "shift-proxy"

[model_providers.shift-proxy]
name = "OpenAI via SHIFT proxy"
base_url = "http://localhost:8787"
requires_openai_auth = true
```

## How It Works

```
Codex CLI
  → openai_base_url = http://localhost:8787
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
shift-ai env --list   # Show configuration for all agents
```
