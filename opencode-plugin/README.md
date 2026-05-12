# @shift-preflight/opencode-plugin

[![npm version](https://img.shields.io/npm/v/@shift-preflight/opencode-plugin)](https://www.npmjs.com/package/@shift-preflight/opencode-plugin)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../LICENSE)

[OpenCode](https://opencode.ai) plugin that auto-starts the [SHIFT](https://github.com/alohaninja/shift) image optimization proxy. Every image-heavy request is transparently optimized before reaching the AI provider — reducing token cost and preventing oversized-image failures.

## Prerequisites

Install the `shift-ai` CLI:

```bash
brew install alohaninja/shift/shift-ai
```

The plugin silently skips if `shift-ai` is not installed — no errors, no breakage.

## Installation

Add the plugin to your `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["@shift-preflight/opencode-plugin"],
  "provider": {
    "anthropic": {
      "options": {
        "baseURL": "http://localhost:8787/v1"
      }
    }
  }
}
```

OpenCode auto-installs npm plugins at startup via Bun. No `npm install` needed.

## How it works

On every OpenCode launch:

1. **Checks prerequisites** — verifies `shift-ai` is on PATH. Silently skips if not installed.
2. **Probes port 8787** — if the SHIFT proxy is already running (from a previous session or another agent), skips startup. Verifies the proxy identity to avoid trusting unrelated services on the same port. Fully idempotent.
3. **Starts the proxy** — spawns the proxy as a detached background process with a sanitized environment (API keys are not passed to the child process).
4. **Verifies startup** — waits briefly to confirm the proxy is healthy. Logs a warning with bypass instructions if it fails.

Startup verification adds ~6 seconds on first launch; subsequent launches detect the running proxy instantly.

The `provider.anthropic.options.baseURL` config routes all Anthropic requests through the proxy. Note: OpenCode's Anthropic client appends only `/messages` (not `/v1/messages`) to the base URL, which is why `/v1` must be included in the `baseURL`. The proxy optimizes images, then forwards to the real Anthropic API. Auth headers and SSE streams pass through unchanged.

## Sharing with other agents

The proxy runs on `localhost:8787` and can be shared with any agent that supports a custom base URL:

```bash
# Claude Code (no /v1 — the Anthropic SDK appends /v1/messages)
ANTHROPIC_BASE_URL=http://localhost:8787 claude

# Codex CLI — add to ~/.codex/config.toml:
# openai_base_url = "http://localhost:8787"
codex

# Gemini CLI (check Gemini CLI docs for the correct env var)
# GEMINI_API_BASE=http://localhost:8787 gemini
```

Once OpenCode starts the proxy, other agents piggyback on it — no need to start it separately.

## Optimization modes

The default mode is `balanced`. To change it, start the proxy manually before OpenCode:

```bash
npx @shift-preflight/runtime proxy --port 8787 --mode economy
```

| Mode | Behavior |
|------|----------|
| **performance** | Minimal transforms. Only enforce hard provider limits. |
| **balanced** | Moderate optimization. Resize oversized images, recompress bloated files. **Default.** |
| **economy** | Aggressive optimization. Downscale to 1024px, minimize token usage. |

## Proxy routes

| Route | Provider |
|-------|----------|
| `POST /v1/messages` | Anthropic |
| `POST /messages` | Anthropic (fallback — rewrites to `/v1/messages`) |
| `POST /v1/chat/completions` | OpenAI |
| `POST /v1beta/models/*` | Google (passthrough only — no image optimization yet) |

## Checking savings

View cumulative token savings across all proxied requests:

```bash
shift-ai gain              # Summary
shift-ai gain --daily      # Day-by-day breakdown
shift-ai gain --format json  # Machine-readable
```

## Upgrading

OpenCode caches npm plugins at `~/.cache/opencode/packages/` and does not automatically check for newer versions. Running `shift-ai setup` will detect and clear stale caches automatically.

To force an upgrade manually:

```bash
# Remove the cached plugin and let OpenCode re-install on next launch
rm -rf ~/.cache/opencode/packages/@shift-preflight*
```

Then restart OpenCode — it will fetch the latest version from npm.

### Version mismatch behavior

When the plugin starts, it probes the running proxy and compares versions:

| Running proxy | Plugin version | Action |
|---------------|---------------|--------|
| Same version | — | Skip (already running) |
| Newer version | — | Skip (don't downgrade) |
| Older version | — | Stop old proxy, start new one |
| No version reported | — | Treat as stale, restart |
| Not running | — | Start proxy |

This means multiple OpenCode sessions with different plugin versions can coexist safely — the newest proxy always wins.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Plugin not loading | Verify `"plugin": ["@shift-preflight/opencode-plugin"]` is in your `opencode.json` |
| Proxy not starting | Check that `shift-ai` is installed: `which shift-ai` |
| Requests failing | Ensure `provider.anthropic.options.baseURL` is set to `http://localhost:8787/v1` and the proxy is running |
| Plugin not updating | OpenCode caches plugins and doesn't auto-update. Run `shift-ai setup` to detect and clear stale caches, or manually: `rm -rf ~/.cache/opencode/packages/@shift-preflight*` |
| Port 8787 in use | Another process is using the port. Check with `lsof -i :8787` |
| "Unknown route" error | Your `baseURL` is likely missing the `/v1` suffix. The correct value for OpenCode is `http://localhost:8787/v1`. OpenCode's Anthropic client appends only `/messages` to the base URL, so `/v1` must be included. |
| Want to bypass proxy | Remove the `baseURL` from your provider config, or stop the proxy |

## License

Apache-2.0 — see [LICENSE](../LICENSE).
