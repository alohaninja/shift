import type { Plugin } from "@opencode-ai/plugin";
import { version as PACKAGE_VERSION } from "./package.json";

const DEFAULT_PORT = 8787;
const PROBE_TIMEOUT_MS = 2_000;
const HEALTH_SERVICE_ID = "@shift-preflight/runtime proxy";

/** Result of probing the running proxy's health endpoint. */
interface ProxyProbeResult {
  /** Whether the proxy is running and healthy. */
  healthy: boolean;
  /** The runtime version reported by the proxy, if available. */
  version?: string;
}

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
 *      "plugin": ["@shift-preflight/opencode-plugin"],
 *      "provider": {
 *        "anthropic": {
 *          "options": {
 *            "baseURL": "http://localhost:8787/v1"
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
 *    running **at the expected version**, skips.
 * 3. If the proxy is running an older version, stops it first.
 * 4. Runs `shift-ai proxy ensure` to start the proxy if needed.
 *
 * The proxy is shared across sessions. Other agents can also use it:
 * ```bash
 * ANTHROPIC_BASE_URL=http://localhost:8787 claude   # Claude Code
 * OPENAI_BASE_URL=http://localhost:8787 codex       # Codex CLI
 * ```
 *
 * @see https://github.com/alohaninja/shift
 */
export const ShiftProxyPlugin: Plugin = async ({ $ }) => {
  const port = DEFAULT_PORT;

  // Bail if shift-ai CLI is not installed
  try {
    await $`which shift-ai`.quiet();
  } catch {
    return {};
  }

  // Check if the SHIFT proxy is already running by probing the health endpoint.
  const probe = await probeShiftProxy(port);

  if (probe.healthy) {
    // Proxy is running — check if it's the version we expect.
    if (probe.version === PACKAGE_VERSION) {
      return {};
    }

    // Version mismatch — stop the old proxy so we can start the new one.
    const old = probe.version ?? "unknown";
    console.log(
      `[shift] proxy version mismatch: running ${old}, expected ${PACKAGE_VERSION} — restarting`,
    );
    try {
      await $`shift-ai proxy stop --quiet`.quiet();
    } catch {
      // Best-effort — proxy ensure will handle port conflicts
    }
  }

  // Use `shift-ai proxy ensure` — handles daemon lifecycle, PID files,
  // health checks, and version-pinned npx spawn internally.
  try {
    await $`shift-ai proxy ensure --quiet`.quiet();

    const postProbe = await probeShiftProxy(port);
    if (postProbe.healthy) {
      console.log(
        `[shift] proxy v${PACKAGE_VERSION} started on port ${port}`,
      );
    } else {
      console.warn(
        `[shift] proxy ensure completed but not yet responding on port ${port}`,
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
 * Returns both health status and the version reported by the proxy.
 */
async function probeShiftProxy(port: number): Promise<ProxyProbeResult> {
  try {
    const res = await fetch(`http://localhost:${port}/health`, {
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    if (!res.ok) return { healthy: false };
    const body = (await res.json()) as {
      service?: string;
      version?: string;
    };
    if (body?.service !== HEALTH_SERVICE_ID) return { healthy: false };
    return { healthy: true, version: body.version };
  } catch {
    return { healthy: false };
  }
}

export default ShiftProxyPlugin;
