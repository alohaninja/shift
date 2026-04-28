import { describe, it, expect, beforeEach, afterEach, mock, spyOn } from "bun:test";
import type { BunShell } from "@opencode-ai/plugin/dist/shell";

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

/** Standard healthy SHIFT proxy response. */
const SHIFT_HEALTH_RESPONSE = { status: "ok", service: "@shift-preflight/runtime proxy" };

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
  // Path 2: shift-ai installed, proxy already running → skip
  // -------------------------------------------------------------------------
  describe("when proxy is already running", () => {
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
  // Path 3: spawn proxy — success
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
        expect.stringContaining("proxy started on port 8787"),
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
  // Path 4: spawn fails entirely
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
