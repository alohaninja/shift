import type { Plugin } from "@opencode-ai/plugin";

const DEFAULT_PORT = 8787;
const DEFAULT_MODE = "balanced";

/**
 * OpenCode plugin that auto-starts the SHIFT preflight proxy.
 *
 * The proxy intercepts AI API requests and optimizes image payloads
 * (resize, recompress, format-convert) before forwarding to the
 * provider API. Transparent to the agent — auth headers and SSE
 * streams pass through unchanged.
 *
 * ## Quick start
 *
 * 1. Install the shift-ai CLI:
 *    ```bash
 *    brew install alohaninja/shift/shift-ai
 *    ```
 *
 * 2. Add the plugin and provider config to `opencode.json`:
 *    ```json
 *    {
 *      "plugin": ["opencode-shift-proxy"],
 *      "provider": {
 *        "anthropic": {
 *          "options": {
 *            "baseURL": "http://localhost:8787"
 *          }
 *        }
 *      }
 *    }
 *    ```
 *
 * 3. Run `opencode` — the proxy starts automatically.
 *
 * ## How it works
 *
 * On startup, the plugin:
 * 1. Checks if `shift-ai` is installed — silently skips if not.
 * 2. Probes `localhost:8787` — if the proxy is already running, skips.
 * 3. Spawns `npx @shift-preflight/runtime proxy` as a detached background
 *    process. The proxy listens on port 8787 and optimizes images in all
 *    requests before forwarding to the upstream API.
 *
 * The proxy is shared across OpenCode sessions. Other agents can also
 * use it by setting the appropriate env var:
 * ```bash
 * ANTHROPIC_BASE_URL=http://localhost:8787 claude   # Claude Code
 * OPENAI_BASE_URL=http://localhost:8787 codex       # Codex CLI
 * ```
 *
 * @see https://github.com/alohaninja/shift
 */
export const ShiftProxyPlugin: Plugin = async ({ $ }) => {
  const port = DEFAULT_PORT;
  const mode = DEFAULT_MODE;

  // Bail if shift-ai CLI is not installed (proxy depends on it)
  try {
    await $`which shift-ai`.quiet();
  } catch {
    return {};
  }

  // Check if proxy is already running by probing the port
  try {
    await fetch(`http://localhost:${port}/`);
    // Any response (even 404) means it's listening
    return {};
  } catch {
    // Not running — start it below
  }

  // Start proxy as a detached background process
  try {
    Bun.spawn(
      [
        "npx",
        "@shift-preflight/runtime",
        "proxy",
        "--port",
        String(port),
        "--mode",
        mode,
      ],
      {
        stdout: "ignore",
        stderr: "ignore",
        stdin: "ignore",
      },
    );
    console.log(`[shift] proxy started on port ${port} (mode: ${mode})`);
  } catch (err) {
    console.log(`[shift] proxy failed to start: ${err}`);
  }

  return {};
};

export default ShiftProxyPlugin;
