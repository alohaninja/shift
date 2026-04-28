import type { Plugin } from "@opencode-ai/plugin";

const DEFAULT_PORT = 8787;
const DEFAULT_MODE = "balanced";
const PROBE_TIMEOUT_MS = 2_000;
const SPAWN_SETTLE_MS = 2_000;
const HEALTH_SERVICE_ID = "@shift-preflight/runtime proxy";

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
 * 2. Probes `localhost:8787/health` — if the SHIFT proxy is already
 *    running, skips. Verifies the service identity to avoid trusting
 *    an unrelated process on the same port.
 * 3. Spawns `npx @shift-preflight/runtime proxy` as a detached
 *    background process. The proxy listens on port 8787 and optimizes
 *    images in all requests before forwarding to the upstream API.
 * 4. Waits briefly to confirm the proxy is healthy before returning.
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

  // Check if the SHIFT proxy is already running by probing the health endpoint.
  // Verify the service identity to avoid trusting an unrelated process on the port.
  if (await isShiftProxyHealthy(port)) {
    return {};
  }

  // Start proxy as a detached background process
  try {
    const proc = Bun.spawn(
      [
        "npx",
        "@shift-preflight/runtime@0.5.1",
        "proxy",
        "--port",
        String(port),
        "--mode",
        mode,
      ],
      {
        detached: true,
        stdout: "ignore",
        stderr: "ignore",
        stdin: "ignore",
      },
    );
    proc.unref();

    // Detect immediate crash vs successful startup
    const exitCode = await Promise.race([
      proc.exited.then((code) => code),
      new Promise<null>((r) => setTimeout(() => r(null), SPAWN_SETTLE_MS)),
    ]);

    if (exitCode !== null) {
      console.warn(`[shift] proxy exited immediately with code ${exitCode}`);
      console.warn(
        `[shift] To bypass, remove baseURL from provider config in opencode.json`,
      );
    } else if (await isShiftProxyHealthy(port)) {
      console.log(`[shift] proxy started on port ${port} (mode: ${mode})`);
    } else {
      console.warn(
        `[shift] proxy spawned but not responding on port ${port} — it may still be starting`,
      );
    }
  } catch (err) {
    console.warn(`[shift] proxy failed to start: ${err}`);
    console.warn(
      `[shift] To bypass, remove baseURL from provider config in opencode.json`,
    );
  }

  return {};
};

/**
 * Probe the SHIFT proxy health endpoint and verify its identity.
 * Returns true only if the response matches the expected service ID,
 * preventing port-squatting by unrelated processes.
 */
async function isShiftProxyHealthy(port: number): Promise<boolean> {
  try {
    const res = await fetch(`http://localhost:${port}/health`, {
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    const body = (await res.json()) as { service?: string };
    return body?.service === HEALTH_SERVICE_ID;
  } catch {
    return false;
  }
}

export default ShiftProxyPlugin;
