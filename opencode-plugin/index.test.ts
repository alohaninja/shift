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
  let originalSpawn: typeof Bun.spawn;

  beforeEach(() => {
    warnSpy = spyOn(console, "warn").mockImplementation(() => {});
    logSpy = spyOn(console, "log").mockImplementation(() => {});
    originalFetch = globalThis.fetch;
    originalSpawn = Bun.spawn;
  });

  afterEach(() => {
    warnSpy.mockRestore();
    logSpy.mockRestore();
    globalThis.fetch = originalFetch;
    Bun.spawn = originalSpawn;
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
    it("returns empty hooks without spawning", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE)),
      );
      globalThis.fetch = fetchMock as any;

      const spawnMock = mock(() => {}) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(hooks).toEqual({});
      expect(fetchMock).toHaveBeenCalled();
      expect(spawnMock).not.toHaveBeenCalled();
    });

    it("does NOT trust a non-SHIFT service on the same port", async () => {
      // All health probes return wrong service identity
      const fetchMock = mock(() =>
        Promise.resolve(jsonResponse({ status: "ok", service: "some-other-app" })),
      );
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Spawn WAS called because the health check didn't match our service
      expect(spawnMock).toHaveBeenCalled();
      // Post-spawn health also returns wrong service → warns
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy spawned but not responding"),
      );
    });
  });

  // -------------------------------------------------------------------------
  // Path 3: proxy running at stale version → restart
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

      // Mock lsof for stopProxy — return a fake PID
      const lsofMockProc = {
        stdout: new ReadableStream({
          start(controller) {
            controller.enqueue(new TextEncoder().encode("12345\n"));
            controller.close();
          },
        }),
        exited: Promise.resolve(0),
      };

      const newProxyProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };

      let spawnCallCount = 0;
      const spawnMock = mock((..._args: any[]) => {
        spawnCallCount++;
        // First spawn call is lsof from stopProxy
        if (spawnCallCount === 1) return lsofMockProc;
        // Second spawn call is the new proxy
        return newProxyProc;
      }) as any;
      Bun.spawn = spawnMock;

      // Mock process.kill so we don't actually kill anything
      const killSpy = spyOn(process, "kill").mockImplementation(() => true);

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Should have logged the version mismatch
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("version mismatch: running 0.6.2"),
      );
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining(`expected ${PACKAGE_VERSION}`),
      );

      // Should have tried to kill the old process
      expect(killSpy).toHaveBeenCalledWith(12345, "SIGTERM");

      // Should have spawned a new proxy
      expect(spawnMock).toHaveBeenCalledTimes(2); // lsof + npx
      const [npxArgs] = spawnMock.mock.calls[1];
      expect(npxArgs[0]).toBe("npx");
      expect(npxArgs[1]).toMatch(/^@shift-preflight\/runtime@/);

      killSpy.mockRestore();
    });

    it("treats proxy with no version field as stale (pre-0.8.0)", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE_NO_VERSION));
        }
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      const lsofMockProc = {
        stdout: new ReadableStream({
          start(controller) {
            controller.enqueue(new TextEncoder().encode("99999\n"));
            controller.close();
          },
        }),
        exited: Promise.resolve(0),
      };

      const newProxyProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };

      let spawnCallCount = 0;
      const spawnMock = mock((..._args: any[]) => {
        spawnCallCount++;
        if (spawnCallCount === 1) return lsofMockProc;
        return newProxyProc;
      }) as any;
      Bun.spawn = spawnMock;

      const killSpy = spyOn(process, "kill").mockImplementation(() => true);

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Should log mismatch with "unknown" version
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("version mismatch: running unknown"),
      );

      // Should have spawned a new proxy after stopping the old one
      expect(spawnMock).toHaveBeenCalledTimes(2);

      killSpy.mockRestore();
    });
  });

  // -------------------------------------------------------------------------
  // Path 4: spawn proxy — success (no proxy running initially)
  // -------------------------------------------------------------------------
  describe("when proxy needs to be started", () => {
    it("spawns with correct args, detached, sanitized env, and calls unref", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          return Promise.reject(new Error("ECONNREFUSED"));
        }
        return Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE));
      });
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(hooks).toEqual({});
      expect(spawnMock).toHaveBeenCalledTimes(1);

      const [args, opts] = spawnMock.mock.calls[0];
      // Version is read from package.json — verify it matches the pattern
      expect(args[0]).toBe("npx");
      expect(args[1]).toMatch(/^@shift-preflight\/runtime@\d+\.\d+\.\d+$/);
      expect(args.slice(2)).toEqual(["proxy", "--port", "8787", "--mode", "balanced"]);

      expect(opts.detached).toBe(true);
      expect(opts.stdout).toBe("ignore");
      expect(opts.stderr).toBe("ignore");
      expect(opts.stdin).toBe("ignore");

      // Env is sanitized — should NOT contain API keys
      expect(opts.env).toBeDefined();
      expect(opts.env.PATH).toBeDefined();
      expect(opts.env.HOME).toBeDefined();
      expect(opts.env.ANTHROPIC_API_KEY).toBeUndefined();
      expect(opts.env.OPENAI_API_KEY).toBeUndefined();

      expect(mockProc.unref).toHaveBeenCalledTimes(1);

      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy v"),
      );
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("started on port 8787"),
      );
    });

    it("warns when proxy exits immediately with nonzero code", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: Promise.resolve(1),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy exited immediately with code 1"),
      );
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("To bypass"),
      );
    });

    it("warns when proxy exits immediately with code 0", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: Promise.resolve(0),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy exited immediately with code 0"),
      );
    });

    it("warns when proxy spawns but health check fails", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy spawned but not responding"),
      );
    });
  });

  // -------------------------------------------------------------------------
  // Path 5: spawn fails entirely
  // -------------------------------------------------------------------------
  describe("when spawn throws", () => {
    it("warns with actionable error message", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const spawnMock = mock(() => {
        throw new Error("ENOENT: npx not found");
      }) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(hooks).toEqual({});
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
      const fetchMock = mock(() =>
        Promise.resolve(jsonResponse({ status: "ok" })),
      );
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(spawnMock).toHaveBeenCalled();
    });

    it("handles non-JSON response gracefully", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(new Response("not json", { status: 200 })),
      );
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(spawnMock).toHaveBeenCalled();
    });

    it("rejects a 500 response even with correct service field", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(jsonResponse(SHIFT_HEALTH_RESPONSE, 500)),
      );
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // 500 should be treated as unhealthy — spawn a new one
      expect(spawnMock).toHaveBeenCalled();
    });

    it("rejects a service identity that is a superstring", async () => {
      const fetchMock = mock(() =>
        Promise.resolve(
          jsonResponse({ status: "ok", service: "@shift-preflight/runtime proxy EXTRA" }),
        ),
      );
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Strict === means superstring doesn't match
      expect(spawnMock).toHaveBeenCalled();
    });
  });
});
