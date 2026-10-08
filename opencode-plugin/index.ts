import type { Plugin } from "@opencode-ai/plugin";
import type { Plugin as V2Plugin } from "@opencode/plugin";
import { execFile } from "node:child_process";
import { version as PACKAGE_VERSION } from "./package.json";

const DEFAULT_PORT = 8787;
const PROBE_TIMEOUT_MS = 2_000;
const COMMAND_TIMEOUT_MS = 15_000;
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
 *      "plugins": ["@shift-preflight/opencode-plugin"],
 *      "providers": {
 *        "anthropic": {
 *          "settings": {
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
 *    running **at the same or newer version**, skips.
 * 3. If the proxy is running an older version, stops it first.
 * 4. Runs `shift-ai proxy ensure` to start the proxy if needed.
 *
 * The proxy is shared across sessions. Other agents can also use it:
 * ```bash
 * ANTHROPIC_BASE_URL=http://localhost:8787 claude   # Claude Code (no /v1 — SDK appends it)
 * # Codex CLI — add to ~/.codex/config.toml: openai_base_url = "http://localhost:8787"
 * ```
 *
 * @see https://github.com/alohaninja/shift
 */

/**
 * Returns true if `running` is >= `required` using semver comparison.
 * Only compares major.minor.patch — pre-release suffixes (e.g. `-beta.1`)
 * are stripped before comparison so `1.0.0-rc.1` is treated as `1.0.0`.
 */
function isVersionAtLeast(running: string, required: string): boolean {
  const parse = (v: string) =>
    v.replace(/-.*$/, "").split(".").map(Number);
  const [rMaj = 0, rMin = 0, rPatch = 0] = parse(running);
  const [pMaj = 0, pMin = 0, pPatch = 0] = parse(required);
  if (rMaj !== pMaj) return rMaj > pMaj;
  if (rMin !== pMin) return rMin > pMin;
  return rPatch >= pPatch;
}

async function run(args: string[]) {
  return new Promise<{ stdout: string }>((resolve, reject) => {
    execFile("shift-ai", args, {
      timeout: args[0] === "--version" ? PROBE_TIMEOUT_MS : COMMAND_TIMEOUT_MS,
      killSignal: "SIGKILL",
    }, (error, stdout) => {
      if (error) reject(error);
      else resolve({ stdout });
    });
  });
}

async function ensureProxy(): Promise<void> {
  const port = DEFAULT_PORT;
  let installedVersion: string | undefined;

  // Bail if shift-ai CLI is not installed
  try {
    const { stdout } = await run(["--version"]);
    installedVersion = /^shift-ai (\d+\.\d+\.\d+(?:-[\w.-]+)?)/m.exec(stdout.toString())?.[1];
  } catch {
    return;
  }

  // Check if the SHIFT proxy is already running by probing the health endpoint.
  const probe = await probeShiftProxy(port);

  if (probe.healthy) {
    // Proxy is running — check if it's at least the version we need.
    // A newer proxy is fine (don't downgrade); only restart if older.
    if (probe.version && isVersionAtLeast(probe.version, PACKAGE_VERSION)) {
      return;
    }

    // `ensure` starts the installed CLI itself; restarting cannot upgrade it.
    if (!installedVersion || !isVersionAtLeast(installedVersion, PACKAGE_VERSION)) {
      console.warn(`[shift] keeping healthy proxy v${probe.version ?? "unknown"}; upgrade shift-ai to ${PACKAGE_VERSION} or newer before restarting`);
      return;
    }

    // Running proxy is older or has no version — stop it so we can start ours.
    const old = probe.version ?? "unknown";
    console.log(
      `[shift] proxy version mismatch: running ${old}, expected ${PACKAGE_VERSION} — restarting`,
    );
    try {
      await run(["proxy", "stop", "--quiet"]);
    } catch (err) {
      // Preserve ownership/migration diagnostics; ensure may reuse the old daemon.
      console.warn(`[shift] could not stop existing proxy: ${err}`);
    }
  }

  // The CLI handles daemon lifecycle, PID files, and startup health checks.
  try {
    await run(["proxy", "ensure", "--quiet"]);

    const postProbe = await probeShiftProxy(port);
    if (postProbe.healthy && postProbe.version && isVersionAtLeast(postProbe.version, PACKAGE_VERSION)) {
      const runningVersion = postProbe.version;
      console.log(
        `[shift] proxy v${runningVersion} started on port ${port}`,
      );
    } else if (postProbe.healthy) {
      console.warn(
        `[shift] keeping healthy proxy v${postProbe.version ?? "unknown"}; automatic restart did not complete (expected ${PACKAGE_VERSION} or newer). Check the CLI diagnostic above before retrying`,
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
}

/** V1 and V2 share bounded subprocess execution. */
export const ShiftProxyPlugin: Plugin = async () => {
  await ensureProxy();
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

// Type-only V2 import keeps the published plugin free of runtime SDK dependencies.
export default {
  id: "shift-preflight",
  async setup() {
    await ensureProxy();
  },
  server: ShiftProxyPlugin,
} satisfies V2Plugin.Plugin & { server: Plugin };
