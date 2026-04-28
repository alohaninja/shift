# SHIFT — Claude Code Integration

Transparent image optimization for Claude Code. Automatically reduces token
costs and prevents oversized-image errors by routing API traffic through the
SHIFT preflight proxy.

## Quick Start (Recommended)

Run the interactive setup:

```bash
shift-ai setup
```

This detects Claude Code and configures everything automatically.

## Manual Setup

### Option A: Shell Profile (Simplest)

Add to your `~/.zshrc` or `~/.bashrc`:

```bash
# Start SHIFT proxy if not running, set Claude Code base URL
eval "$(shift-ai env claude-code)"
```

Then ensure the proxy is running (via LaunchAgent or manually):

```bash
shift-ai proxy start
```

### Option B: Claude Code Settings

Create or edit `~/.claude/settings.json`:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:8787"
  }
}
```

Then start the proxy:

```bash
shift-ai proxy start
```

### Option C: Claude Code Hook (Per-Session Ensure)

Create `~/.claude/hooks.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "shift-ai proxy ensure --quiet"
          }
        ]
      }
    ]
  }
}
```

This ensures the proxy is running at the start of each Claude Code session.
Combined with Option A or B for the base URL, this is fully automatic.

### Option D: LaunchAgent (Always-On)

Install a macOS LaunchAgent so the proxy starts on login:

```bash
shift-ai setup
# Or manually:
# shift-ai proxy start  (the setup command handles LaunchAgent installation)
```

## How It Works

```
Claude Code
  → ANTHROPIC_BASE_URL=http://localhost:8787
    → SHIFT Proxy (inspects + optimizes images)
      → https://api.anthropic.com/v1/messages
```

1. Claude Code sends API requests to `localhost:8787` instead of the Anthropic API
2. The SHIFT proxy inspects image payloads and optimizes oversized images
3. Optimized requests are forwarded to the real Anthropic API
4. Responses (including SSE streams) pass through unchanged

Auth headers pass through untouched — the proxy never stores credentials.

## Verification

Check that the proxy is running:

```bash
shift-ai proxy status
```

View cumulative token savings:

```bash
shift-ai gain
```

## Shared Proxy

The proxy on port 8787 is shared across all agents. If you also use OpenCode,
Codex CLI, or other agents, they can all use the same proxy:

```bash
shift-ai env --list   # Show env vars for all agents
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Connection refused` | Run `shift-ai proxy start` |
| Images still large | Check `shift-ai proxy status` — must show "running" |
| Claude Code ignores base URL | Verify `~/.claude/settings.json` or env var is set |
| Port 8787 in use by another process | The health check verifies service identity; another SHIFT proxy is likely already running |
