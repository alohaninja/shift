import { describe, it, expect, beforeEach, afterEach, mock, spyOn } from "bun:test";
import type { BunShell } from "@opencode-ai/plugin/dist/shell";
import { version as PACKAGE_VERSION } from "./package.json";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Build a mock `$` tagged-template shell that resolves or rejects. */
function createMockShell(behavior: "resolve" | "reject"): BunShell {
  const shellFn = (() => {
    const promise =
      behavior === "resolve"
        ? Promise.resolve({ exitCode: 0, stdout: Buffer.from(""), stderr: Buffer.from("") })
        : Promise.reject(new Error("not found"));

    // The plugin calls $`which shift-ai`.quiet(), so the returned promise
    // needs a .quiet() method that returns itself.
    (promise as any).quiet = () => promise;
    return promise;
  }) as unknown as BunShell;
  return shellFn;
}

/**
 * Build a mock shell that tracks the commands it receives and lets
 * each command resolve or reject independently.
 *
 * Returns the shell function and an array of captured command strings.
 */
function createTrackingShell(
  commandResults: Record<string, "resolve" | "reject"> = {},
) {
  const calls: string[] = [];

  const shellFn = ((strings: TemplateStringsArray, ...values: unknown[]) => {
    // Reconstruct the template literal into a single command string
    let cmd = strings[0];
    for (let i = 0; i < values.length; i++) {
      cmd += String(values[i]) + strings[i + 1];
    }
    calls.push(cmd);

    // Determine result based on command content
    let shouldReject = false;
    for (const [pattern, result] of Object.entries(commandResults)) {
      if (cmd.includes(pattern)) {
        shouldReject = result === "reject";
        break;
      }
    }

    const promise = shouldReject
      ? Promise.reject(new Error(`command failed: ${cmd}`))
      : Promise.resolve({ exitCode: 0, stdout: Buffer.from(""), stderr: Buffer.from("") });

    (promise as any).quiet = () => promise;
    return promise;
  }) as unknown as BunShell;

  return { shell: shellFn, calls };
}

/** Build a minimal PluginInput with the given shell. */
function createPluginInput(shell: BunShell) {
  return {
    $: shell,
    client: {} as any,
    project: {} as any,
    directory: "/tmp",
    worktree: "/tmp",
  };
}

/** Create a mock Response with JSON body. */
function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Standard healthy SHIFT proxy response with current version. */
const SHIFT_HEALTH_RESPONSE = {
  status: "ok",
  service: "@shift-preflight/runtime proxy",
  version: PACKAGE_VERSION,
};

/** Healthy response from an older proxy version (no version field). */
const SHIFT_HEALTH_RESPONSE_NO_VERSION = {
  status: "ok",
  service: "@shift-preflight/runtime proxy",
};

/** Healthy response from a stale proxy version. */
const SHIFT_HEALTH_RESPONSE_STALE = {
  status: "ok",
  service: "@shift-preflight/runtime proxy",
  version: "0.6.2",
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("ShiftProxyPlugin", () => {
  let warnSpy: ReturnType<typeof spyOn>;
  let logSpy: ReturnType<typeof spyOn>;
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    warnSpy = spyOn(console, "warn").mockImplementation(() => {});
    logSpy = spyOn(console, "log").mockImplementation(() => {});
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    warnSpy.mockRestore();
    logSpy.mockRestore();
    globalThis.fetch = originalFetch;
  });

  // -------------------------------------------------------------------------
  // Path 1: shift-ai not installed → skip silently
  // -------------------------------------------------------------------------
  describe("when shift-ai is not installed", () => {
    it("returns empty hooks without probing the port", async () => {
      const fetchMock = mock(() => Promise.resolve(jsonResponse({ status: "ok" })));
      globalThis.fetch = fetchMock as any;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("reject")));

      expect(hooks).toEqual({});
      // fetch should never be called — we bail before the port probe
      expect(fetchMock).not.toHaveBeenCalled();
    });
  });

  // -------------------------------------------------------------------------
  // Path 2: shift-ai installed, proxy already running at correct version → skip
  // -------------------------------------------------------------------------
  describe("when proxy is already running at correct version", () => {
    it("returns empty hooks without calling proxy ensure", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE)),
      );
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(shell));

      expect(hooks).toEqual({});
      expect(fetchMock).toHaveBeenCalled();
      // Should only have called `which shift-ai`, not `proxy ensure`
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(false);
    });

    it("does NOT trust a non-SHIFT service on the same port", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        // Both probes return a non-SHIFT service
        return Promise.resolve(
          jsonResponse({ status: "ok", service: "some-other-app" }),
        );
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // Should have called proxy ensure since the health check didn't match
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
      // Post-ensure health also returns wrong service → warns
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy ensure completed but not yet responding"),
      );
    });
  });

  // -------------------------------------------------------------------------
  // Path 3: proxy running at stale version → stop then ensure
  // -------------------------------------------------------------------------
  describe("when proxy is running at a stale version", () => {
    it("logs version mismatch and restarts the proxy", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        // First probe: old version running
        if (fetchCallCount === 1) {
          return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE_STALE));
        }
        // Post-restart probe: new version healthy
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // Should have logged the version mismatch
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("version mismatch: running 0.6.2"),
      );
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining(`expected ${PACKAGE_VERSION}`),
      );

      // Should have called proxy stop then proxy ensure
      expect(calls.some((c) => c.includes("proxy stop"))).toBe(true);
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);

      // Stop should come before ensure
      const stopIdx = calls.findIndex((c) => c.includes("proxy stop"));
      const ensureIdx = calls.findIndex((c) => c.includes("proxy ensure"));
      expect(stopIdx).toBeLessThan(ensureIdx);

      // Should log successful start
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining(`proxy v${PACKAGE_VERSION} started on port`),
      );
    });

    it("treats proxy with no version field as stale", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE_NO_VERSION));
        }
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // Should log mismatch with "unknown" version
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("version mismatch: running unknown"),
      );

      // Should have called stop then ensure
      expect(calls.some((c) => c.includes("proxy stop"))).toBe(true);
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });

    it("proceeds with ensure even if stop fails", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE_STALE));
        }
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      // Make proxy stop fail — ensure should still proceed
      const { shell, calls } = createTrackingShell({ "proxy stop": "reject" });

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // Stop was attempted even though it failed
      expect(calls.some((c) => c.includes("proxy stop"))).toBe(true);
      // Ensure was still called
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });
  });

  // -------------------------------------------------------------------------
  // Path 4: no proxy running → start via ensure
  // -------------------------------------------------------------------------
  describe("when proxy needs to be started", () => {
    it("calls proxy ensure and logs success when health check passes", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          return Promise.reject(new Error("ECONNREFUSED"));
        }
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(shell));

      expect(hooks).toEqual({});
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
      // Should NOT call stop since nothing was running
      expect(calls.some((c) => c.includes("proxy stop"))).toBe(false);

      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining(`proxy v${PACKAGE_VERSION} started on port 8787`),
      );
    });

    it("warns when proxy ensure succeeds but health check still fails", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy ensure completed but not yet responding"),
      );
    });
  });

  // -------------------------------------------------------------------------
  // Path 5: proxy ensure fails entirely
  // -------------------------------------------------------------------------
  describe("when proxy ensure fails", () => {
    it("warns with actionable error message", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell({
        "proxy ensure": "reject",
      });

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(shell));

      expect(hooks).toEqual({});
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy failed to start"),
      );
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("To bypass"),
      );
    });
  });

  // -------------------------------------------------------------------------
  // Health probe edge cases
  // -------------------------------------------------------------------------
  describe("health probe edge cases", () => {
    it("rejects a response with no service field", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        // Both probes return no service field
        return Promise.resolve(jsonResponse({ status: "ok" }));
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });

    it("handles non-JSON response gracefully", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(new Response("not json", { status: 200 })),
      );
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });

    it("rejects a 500 response even with correct service field", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE, fetchCallCount === 1 ? 500 : 200));
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // 500 should be treated as unhealthy — call ensure
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });

    it("rejects a service identity that is a superstring", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        return Promise.resolve(
          jsonResponse(
            fetchCallCount === 1
              ? { status: "ok", service: "@shift-preflight/runtime proxy EXTRA" }
              : SHIFT_HEALTH_RESPONSE,
          ),
        );
      });
      globalThis.fetch = fetchMock as any;

      const { shell, calls } = createTrackingShell();

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(shell));

      // Strict === means superstring doesn't match
      expect(calls.some((c) => c.includes("proxy ensure"))).toBe(true);
    });
  });
});
