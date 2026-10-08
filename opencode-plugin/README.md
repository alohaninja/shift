# @shift-preflight/opencode-plugin

[![npm version](https://img.shields.io/npm/v/@shift-preflight/opencode-plugin)](https://www.npmjs.com/package/@shift-preflight/opencode-plugin)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../LICENSE)

[OpenCode](https://opencode.ai) plugin that auto-starts the [SHIFT](https://github.com/alohaninja/shift) image optimization proxy. Every image-heavy request is transparently optimized before reaching the AI provider — reducing token cost and preventing oversized-image failures.

## Prerequisites

Supports OpenCode V2 (validated on 2.0.22) and V1 1.18.29+. Older V1 releases
require the previous plugin release, `@shift-preflight/opencode-plugin@0.10.2`.

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
  "plugins": ["@shift-preflight/opencode-plugin"],
  "providers": {
    "anthropic": {
      "settings": {
        "baseURL": "http://localhost:8787/v1"
      }
    }
  }
}
```

OpenCode installs npm plugins automatically. No `npm install` needed. For V1,
use `plugin`, `provider`, and `options` instead of `plugins`, `providers`, and
`settings`. V2 also accepts these V1 configuration keys.

## How it works

When OpenCode loads the plugin for a location:

1. **Checks prerequisites** — verifies `shift-ai` is on PATH. Silently skips if not installed.
2. **Probes port 8787** — if the SHIFT proxy is already running (from a previous session or another agent), skips startup. Verifies the proxy identity to avoid trusting unrelated services on the same port. Fully idempotent.
3. **Starts the proxy** — calls `shift-ai proxy ensure --quiet`; the CLI manages the shared native proxy daemon.
4. **Verifies startup** — waits briefly to confirm the proxy is healthy. Logs a warning with bypass instructions if it fails.

Both entrypoints use Node's process API with a two-second CLI version deadline
and a fifteen-second lifecycle-command deadline. Timed-out commands are killed.
The proxy outlives the plugin, so unloading a location does not stop other agents' traffic.

The `providers.anthropic.settings.baseURL` config routes Anthropic requests through
the proxy. Include `/v1` in the base URL. The proxy optimizes images, then forwards
to the Anthropic API. Auth headers and SSE streams pass through unchanged.

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
shift-ai proxy start --port 8787 --mode economy
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

Running `shift-ai setup` detects older cached `@latest` copies in both OpenCode
layouts: V1's `~/.cache/opencode/packages/` and V2's timestamped generations under
`~/.cache/opencode/npm/`. It respects `XDG_CACHE_HOME` and keeps current, newer,
and explicitly pinned versions.

To force an upgrade manually:

```bash
# V2: remove only this plugin's @latest cache, then restart OpenCode
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/opencode/npm/@shift-preflight/opencode-plugin@latest"
```

Then restart OpenCode — it will fetch the latest version from npm.

### Version mismatch behavior

When the plugin starts, it probes the running proxy and compares versions:

| Running proxy vs. plugin version | Installed CLI | Action |
|---------------|---------------|--------|
| Same version | — | Skip (already running) |
| Newer version | — | Skip (don't downgrade) |
| Older version | Installed CLI meets plugin version | Stop old proxy, start installed CLI's proxy |
| Older version | Installed CLI too old or version unknown | Keep healthy proxy; warn to upgrade `shift-ai` |
| No version reported | Installed CLI meets plugin version | Treat as stale, restart |
| No version reported | Installed CLI too old or version unknown | Keep healthy proxy; warn to upgrade `shift-ai` |
| Not running | — | Start proxy |

The plugin never downgrades a healthy newer proxy. Upgrade the `shift-ai` CLI
alongside the plugin: `proxy ensure` starts the installed binary, so a newer
plugin alone cannot upgrade an older daemon.

If the CLI reports a legacy PID-only state file, automatic restart is refused
until the old daemon is stopped and its legacy state removed. Follow the
[one-time daemon migration](../README.md#daemon-ownership-and-upgrades); the plugin
can continue using the healthy old proxy in the meantime.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Plugin not loading | Verify `"plugins": ["@shift-preflight/opencode-plugin"]` is in your V2 `opencode.json`; V1 uses `plugin` |
| Proxy not starting | Check that `shift-ai` is installed: `which shift-ai` |
| Requests failing | Ensure `providers.anthropic.settings.baseURL` is set to `http://localhost:8787/v1` and the proxy is running |
| Plugin not updating | Run an updated `shift-ai setup` to clear stale `@latest` copies, or use the cache command above |
| Port 8787 in use | Another process is using the port. Check with `lsof -i :8787` |
| "Unknown route" error | Your `baseURL` is likely missing the `/v1` suffix. The correct value for OpenCode is `http://localhost:8787/v1`. OpenCode's Anthropic client appends only `/messages` to the base URL, so `/v1` must be included. |
| Want to bypass proxy | Remove the `baseURL` from your provider config, or stop the proxy |

## License

Apache-2.0 — see [LICENSE](../LICENSE).
