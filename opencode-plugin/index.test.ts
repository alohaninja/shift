import { describe, it, expect, beforeEach, mock, spyOn } from "bun:test";
import type { BunShell, BunShellPromise } from "@opencode-ai/plugin/dist/shell";

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// We import the plugin fresh in each describe block after setting up mocks.
// Bun.spawn is a property on the global Bun object so we mock it directly.

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

  // Restore after each test
  function restore() {
    warnSpy.mockRestore();
    logSpy.mockRestore();
    globalThis.fetch = originalFetch;
    Bun.spawn = originalSpawn;
  }

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

      restore();
    });
  });

  // -------------------------------------------------------------------------
  // Path 2: shift-ai installed, proxy already running → skip
  // -------------------------------------------------------------------------
  describe("when proxy is already running", () => {
    it("returns empty hooks without spawning", async () => {
      // Health endpoint returns the expected service ID
      const fetchMock = mock(() =>
        Promise.resolve(
          jsonResponse({ status: "ok", service: "@shift-preflight/runtime proxy" }),
        ),
      );
      globalThis.fetch = fetchMock as any;

      const spawnMock = mock(() => {}) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(hooks).toEqual({});
      // fetch was called (health probe), but spawn was NOT called
      expect(fetchMock).toHaveBeenCalled();
      expect(spawnMock).not.toHaveBeenCalled();

      restore();
    });

    it("does NOT trust a non-SHIFT service on the same port", async () => {
      let fetchCallCount = 0;
      // First call: health probe returns wrong service → not our proxy
      // Second call (post-spawn health check): also returns wrong service
      const fetchMock = mock(() => {
        fetchCallCount++;
        return Promise.resolve(jsonResponse({ status: "ok", service: "some-other-app" }));
      });
      globalThis.fetch = fetchMock as any;

      // Spawn should be called since health check fails identity verification
      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}), // never resolves (simulates long-running process)
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Spawn WAS called because the health check didn't match our service
      expect(spawnMock).toHaveBeenCalled();

      restore();
    });
  });

  // -------------------------------------------------------------------------
  // Path 3: spawn proxy — success
  // -------------------------------------------------------------------------
  describe("when proxy needs to be started", () => {
    it("spawns with correct args, detached, and calls unref", async () => {
      let fetchCallCount = 0;
      const fetchMock = mock(() => {
        fetchCallCount++;
        if (fetchCallCount === 1) {
          // First call: health probe — proxy not running
          return Promise.reject(new Error("ECONNREFUSED"));
        }
        // Second call: post-spawn health check — proxy is now up
        return Promise.resolve(
          jsonResponse({ status: "ok", service: "@shift-preflight/runtime proxy" }),
        );
      });
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}), // never exits (long-running server)
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      const hooks = await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(hooks).toEqual({});

      // Verify spawn args
      expect(spawnMock).toHaveBeenCalledTimes(1);
      const [args, opts] = spawnMock.mock.calls[0];
      expect(args).toEqual([
        "npx",
        "@shift-preflight/runtime@0.5.1",
        "proxy",
        "--port",
        "8787",
        "--mode",
        "balanced",
      ]);
      expect(opts.detached).toBe(true);
      expect(opts.stdout).toBe("ignore");
      expect(opts.stderr).toBe("ignore");
      expect(opts.stdin).toBe("ignore");

      // unref was called
      expect(mockProc.unref).toHaveBeenCalledTimes(1);

      // Success log was printed
      expect(logSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy started on port 8787"),
      );

      restore();
    });

    it("warns when proxy exits immediately", async () => {
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: Promise.resolve(1), // exits immediately with code 1
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

      restore();
    });

    it("warns when proxy spawns but health check fails", async () => {
      // All fetch calls fail (connection refused)
      const fetchMock = mock(() => Promise.reject(new Error("ECONNREFUSED")));
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}), // doesn't exit
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy spawned but not responding"),
      );

      restore();
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

      // Still returns empty hooks (doesn't throw)
      expect(hooks).toEqual({});

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("proxy failed to start"),
      );
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining("To bypass"),
      );

      restore();
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
        // Health endpoint returns JSON without service field
        return Promise.resolve(jsonResponse({ status: "ok" }));
      });
      globalThis.fetch = fetchMock as any;

      const mockProc = {
        unref: mock(() => {}),
        exited: new Promise<number>(() => {}),
      };
      const spawnMock = mock(() => mockProc) as any;
      Bun.spawn = spawnMock;

      const { ShiftProxyPlugin } = await import("./index");
      await ShiftProxyPlugin(createPluginInput(createMockShell("resolve")));

      // Should have attempted to spawn since health probe didn't match
      expect(spawnMock).toHaveBeenCalled();

      restore();
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

      // Non-JSON response should be treated as "not our proxy" → spawn
      expect(spawnMock).toHaveBeenCalled();

      restore();
    });
  });
});
