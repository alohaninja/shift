import type { Plugin } from "@opencode-ai/plugin";
import { version as PACKAGE_VERSION } from "./package.json";

const DEFAULT_PORT = 8787;
const DEFAULT_MODE = "balanced";
const PROBE_TIMEOUT_MS = 2_000;
const SPAWN_SETTLE_MS = 2_000;
const HEALTH_SERVICE_ID = "@shift-preflight/runtime proxy";
const RUNTIME_PACKAGE = `@shift-preflight/runtime@${PACKAGE_VERSION}`;

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
 * 3. If the proxy is running an older version, kills it gracefully
 *    before spawning the new one.
 * 4. Spawns `npx @shift-preflight/runtime proxy` as a detached
 *    background process. The proxy listens on port 8787 and optimizes
 *    images in all requests before forwarding to the upstream API.
 * 5. Waits briefly to confirm the proxy is healthy before returning.
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
  const probe = await probeShiftProxy(port);

  if (probe.healthy) {
    // Proxy is running — check if it's the version we expect.
    // If the proxy doesn't report a version (pre-0.8.0), treat it as stale
    // since version-aware health was added in 0.8.0.
    if (probe.version === PACKAGE_VERSION) {
      return {};
    }

    // Version mismatch — stop the old proxy so we can start the new one.
    const old = probe.version ?? "unknown";
    console.log(
      `[shift] proxy version mismatch: running ${old}, expected ${PACKAGE_VERSION} — restarting`,
    );
    await stopProxy(port);
  }

  // Start proxy as a detached background process.
  // Only pass PATH/HOME/SHELL to the child — the proxy doesn't need API keys
  // and we don't want npm lifecycle scripts to have access to them.
  try {
    const proc = Bun.spawn(
      [
        "npx",
        RUNTIME_PACKAGE,
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
        env: {
          PATH: process.env.PATH ?? "",
          HOME: process.env.HOME ?? "",
          SHELL: process.env.SHELL ?? "",
          TMPDIR: process.env.TMPDIR ?? "",
          npm_config_registry: process.env.npm_config_registry ?? "",
        },
      },
    );
    proc.unref();

    // Detect immediate crash vs successful startup.
    // Clear the timer when proc.exited wins the race to avoid leaking it.
    let settleTimer: ReturnType<typeof setTimeout>;
    const exitCode = await Promise.race([
      proc.exited.then((code) => {
        clearTimeout(settleTimer);
        return code;
      }),
      new Promise<null>((r) => {
        settleTimer = setTimeout(() => r(null), SPAWN_SETTLE_MS);
      }),
    ]);

    if (exitCode !== null) {
      console.warn(`[shift] proxy exited immediately with code ${exitCode}`);
      console.warn(
        `[shift] To bypass, remove baseURL from provider config in opencode.json`,
      );
    } else {
      const postProbe = await probeShiftProxy(port);
      if (postProbe.healthy) {
        console.log(`[shift] proxy v${PACKAGE_VERSION} started on port ${port} (mode: ${mode})`);
      } else {
        console.warn(
          `[shift] proxy spawned but not responding on port ${port} — it may still be starting`,
        );
      }
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

/**
 * Gracefully stop a running SHIFT proxy by sending a request to
 * an endpoint that triggers shutdown, or by finding and killing
 * the process listening on the port.
 */
async function stopProxy(port: number): Promise<void> {
  try {
    // Find the PID of the process listening on the port and kill it.
    // This works on macOS and Linux.
    const proc = Bun.spawn(["lsof", "-ti", `tcp:${port}`], {
      stdout: "pipe",
      stderr: "ignore",
    });
    const output = await new Response(proc.stdout).text();
    const pids = output
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((s) => parseInt(s, 10))
      .filter((n) => !isNaN(n));

    for (const pid of pids) {
      try {
        process.kill(pid, "SIGTERM");
      } catch {
        // Process may have already exited
      }
    }

    // Wait briefly for the port to free up
    if (pids.length > 0) {
      await new Promise((r) => setTimeout(r, 1_000));
    }
  } catch {
    // Best-effort — if we can't stop it, the new spawn will fail on port conflict
    // and the user will see a clear error message.
  }
}

export default ShiftProxyPlugin;
