import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { readFile, unlink, mkdir, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes } from "node:crypto";
import {
  buildRunRecord,
  recordRun,
  getSessionStats,
  resetSessionStats,
  defaultStatsPath,
  _resetNodeDeps,
  type RunRecord,
} from "../../src/core/stats.js";

// Use a unique temp file for each test to avoid collisions
function tmpStatsPath(): string {
  const id = randomBytes(6).toString("hex");
  return join(tmpdir(), `shift-test-stats-${id}.jsonl`);
}

describe("buildRunRecord", () => {
  it("creates a record with correct fields when bytes were saved", () => {
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: 150,
      source: "proxy",
    });

    expect(record.provider).toBe("anthropic");
    expect(record.bytes_before).toBe(100_000);
    expect(record.bytes_after).toBe(40_000);
    expect(record.duration_ms).toBe(150);
    expect(record.images).toBe(1);
    expect(record.modified).toBe(1);
    expect(record.source).toBe("proxy");
    expect(record.action_counts).toEqual([["optimize", 1]]);
    expect(record.timestamp).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(record.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("creates a zero-impact record when no bytes were saved", () => {
    const record = buildRunRecord({
      provider: "openai",
      originalBytes: 5_000,
      optimizedBytes: 5_000,
      durationMs: 10,
      source: "proxy",
    });

    expect(record.images).toBe(0);
    expect(record.modified).toBe(0);
    expect(record.action_counts).toEqual([]);
  });

  it("sets token_savings to zeros (proxy doesn't have per-image token data)", () => {
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: 150,
    });

    expect(record.token_savings).toEqual({
      openai_before: 0,
      openai_after: 0,
      anthropic_before: 0,
      anthropic_after: 0,
    });
  });

  it("clamps negative duration_ms to zero", () => {
    // Negative duration can happen from NTP clock adjustments
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: -50,
      source: "proxy",
    });

    expect(record.duration_ms).toBe(0);
  });
});

describe("recordRun", () => {
  let statsPath: string;

  beforeEach(() => {
    statsPath = tmpStatsPath();
  });

  afterEach(async () => {
    await unlink(statsPath).catch(() => {});
  });

  it("writes a valid JSON line to the stats file", async () => {
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 80_000,
      optimizedBytes: 30_000,
      durationMs: 200,
      source: "proxy",
    });

    await recordRun(record, statsPath);

    const content = await readFile(statsPath, "utf-8");
    const lines = content.trim().split("\n");
    expect(lines).toHaveLength(1);

    const parsed: RunRecord = JSON.parse(lines[0]);
    expect(parsed.provider).toBe("anthropic");
    expect(parsed.bytes_before).toBe(80_000);
    expect(parsed.bytes_after).toBe(30_000);
    expect(parsed.source).toBe("proxy");
  });

  it("appends multiple records", async () => {
    const r1 = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 50_000,
      durationMs: 100,
      source: "proxy",
    });
    const r2 = buildRunRecord({
      provider: "openai",
      originalBytes: 200_000,
      optimizedBytes: 80_000,
      durationMs: 250,
      source: "proxy",
    });

    await recordRun(r1, statsPath);
    await recordRun(r2, statsPath);

    const content = await readFile(statsPath, "utf-8");
    const lines = content.trim().split("\n");
    expect(lines).toHaveLength(2);

    expect(JSON.parse(lines[0]).provider).toBe("anthropic");
    expect(JSON.parse(lines[1]).provider).toBe("openai");
  });

  it("creates the parent directory if it doesn't exist", async () => {
    const deepPath = join(
      tmpdir(),
      `shift-test-nested-${randomBytes(4).toString("hex")}`,
      "stats.jsonl",
    );
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    await recordRun(record, deepPath);

    const content = await readFile(deepPath, "utf-8");
    expect(content.trim().length).toBeGreaterThan(0);

    // Cleanup
    await unlink(deepPath).catch(() => {});
  });

  it("does not throw on write failure and logs a warning", async () => {
    // Point at a path that's a directory (can't write to a directory)
    const dirPath = join(tmpdir(), `shift-test-dir-${randomBytes(4).toString("hex")}`);
    await mkdir(dirPath, { recursive: true });

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    // Should not throw
    await expect(recordRun(record, dirPath)).resolves.toBeUndefined();

    // Should have logged a warning
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("[shift-runtime] Failed to write stats:"),
    );

    warnSpy.mockRestore();
  });

  it("rejects symlinks and logs a warning", async () => {
    // Create a real file, then a symlink pointing to it
    const realPath = tmpStatsPath();
    const symlinkPath = tmpStatsPath();

    // Create the target file
    const { appendFile: appendFileFs } = await import("node:fs/promises");
    await appendFileFs(realPath, "", { mode: 0o600 });
    // Create a symlink to it
    await symlink(realPath, symlinkPath);

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    await recordRun(record, symlinkPath);

    // Should have warned about the symlink
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("is a symlink"),
    );

    // The real file should NOT have been written to
    const content = await readFile(realPath, "utf-8");
    expect(content).toBe("");

    warnSpy.mockRestore();
    await unlink(symlinkPath).catch(() => {});
    await unlink(realPath).catch(() => {});
  });
});

describe("session stats", () => {
  beforeEach(() => {
    resetSessionStats();
  });

  it("starts with zeros", () => {
    const stats = getSessionStats();
    expect(stats.totalRequests).toBe(0);
    expect(stats.totalImages).toBe(0);
    expect(stats.totalBytesSaved).toBe(0);
  });

  it("accumulates across multiple recordRun calls", async () => {
    const statsPath = tmpStatsPath();

    const r1 = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: 100,
      source: "proxy",
    });
    const r2 = buildRunRecord({
      provider: "openai",
      originalBytes: 200_000,
      optimizedBytes: 80_000,
      durationMs: 200,
      source: "proxy",
    });

    await recordRun(r1, statsPath);
    await recordRun(r2, statsPath);

    const stats = getSessionStats();
    expect(stats.totalRequests).toBe(2);
    expect(stats.totalImages).toBe(2); // both had savings
    expect(stats.totalImagesModified).toBe(2);
    expect(stats.totalBytesSaved).toBe(180_000); // 60K + 120K

    await unlink(statsPath).catch(() => {});
  });

  it("returns a copy (not a reference to internal state)", () => {
    const stats1 = getSessionStats();
    const stats2 = getSessionStats();
    expect(stats1).not.toBe(stats2);
    expect(stats1.tokenSavings).not.toBe(stats2.tokenSavings);
  });

  it("still accumulates session stats even when file write is skipped (symlink)", async () => {
    // Session accumulation happens before the file write, so even if
    // the write is rejected (e.g., symlink), session stats are updated.
    const realPath = tmpStatsPath();
    const symlinkPath = tmpStatsPath();
    const { appendFile: appendFileFs } = await import("node:fs/promises");
    await appendFileFs(realPath, "", { mode: 0o600 });
    await symlink(realPath, symlinkPath);

    vi.spyOn(console, "warn").mockImplementation(() => {});

    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: 100,
      source: "proxy",
    });

    await recordRun(record, symlinkPath);

    const stats = getSessionStats();
    expect(stats.totalRequests).toBe(1);
    expect(stats.totalBytesSaved).toBe(60_000);

    vi.restoreAllMocks();
    await unlink(symlinkPath).catch(() => {});
    await unlink(realPath).catch(() => {});
  });
});

// ────────────────────────────────────────────────────────────────────
// Lazy-load / non-Node fallback tests
// ────────────────────────────────────────────────────────────────────

describe("lazy-load Node deps", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("_resetNodeDeps forces re-load on next call", async () => {
    // First call loads deps successfully (we're in Node)
    const statsPath = tmpStatsPath();
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    await recordRun(record, statsPath);
    const content = await readFile(statsPath, "utf-8");
    expect(content.trim().length).toBeGreaterThan(0);

    // Reset and verify it works again (deps are re-loaded)
    _resetNodeDeps();
    const statsPath2 = tmpStatsPath();
    await recordRun(record, statsPath2);
    const content2 = await readFile(statsPath2, "utf-8");
    expect(content2.trim().length).toBeGreaterThan(0);

    await unlink(statsPath).catch(() => {});
    await unlink(statsPath2).catch(() => {});
  });

  it("defaultStatsPath returns a non-empty path in Node", async () => {
    _resetNodeDeps();
    const path = await defaultStatsPath();
    expect(path).toMatch(/\.shift[/\\]stats\.jsonl$/);
    expect(path.length).toBeGreaterThan(0);
  });

  it("concurrent recordRun calls share the same load (no double-load race)", async () => {
    _resetNodeDeps();
    resetSessionStats();

    const statsPath = tmpStatsPath();
    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    // Fire 5 concurrent recordRun calls — all should succeed
    await Promise.all(
      Array.from({ length: 5 }, () => recordRun(record, statsPath)),
    );

    const stats = getSessionStats();
    expect(stats.totalRequests).toBe(5);

    const content = await readFile(statsPath, "utf-8");
    const lines = content.trim().split("\n");
    expect(lines).toHaveLength(5);

    await unlink(statsPath).catch(() => {});
  });
});

describe("non-Node fallback (simulated)", () => {
  beforeEach(() => {
    _resetNodeDeps();
    resetSessionStats();
  });

  afterEach(() => {
    _resetNodeDeps(); // restore to undefined so subsequent tests re-load real deps
    vi.restoreAllMocks();
  });

  it("recordRun accumulates session stats but skips file write when deps unavailable", async () => {
    // Mock dynamic import to simulate non-Node environment
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    // We need to force loadNodeDeps to fail. We do this by:
    // 1. Resetting deps (already done in beforeEach)
    // 2. Mocking the global import to reject for node: specifiers
    //    Since vitest runs in Node where dynamic import() always succeeds
    //    for node: modules, we instead directly test the contract by
    //    importing the module fresh with mocked deps.
    //
    // The most reliable approach: call _resetNodeDeps, then mock
    // the "node:fs/promises" module to throw on import, and verify
    // recordRun still accumulates.

    // Use vi.mock to make node:fs/promises throw
    vi.doMock("node:fs/promises", () => {
      throw new Error("Simulated: node:fs/promises unavailable");
    });

    // Force re-import to pick up the mock
    const { recordRun: recordRunFresh, getSessionStats: getStatsFresh, _resetNodeDeps: resetFresh } =
      await import("../../src/core/stats.js");
    resetFresh();

    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 100_000,
      optimizedBytes: 40_000,
      durationMs: 100,
      source: "proxy",
    });

    const statsPath = tmpStatsPath();
    await recordRunFresh(record, statsPath);

    // Session stats should still accumulate
    const stats = getStatsFresh();
    expect(stats.totalRequests).toBe(1);
    expect(stats.totalBytesSaved).toBe(60_000);

    // File should NOT have been created
    const exists = await readFile(statsPath, "utf-8").then(
      () => true,
      () => false,
    );
    expect(exists).toBe(false);

    // Should have warned about unavailable Node APIs
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("Node APIs unavailable"),
    );

    vi.doUnmock("node:fs/promises");
  });

  it("defaultStatsPath returns empty string when deps unavailable", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});

    vi.doMock("node:fs/promises", () => {
      throw new Error("Simulated: node:fs/promises unavailable");
    });

    const { defaultStatsPath: defaultPathFresh, _resetNodeDeps: resetFresh } =
      await import("../../src/core/stats.js");
    resetFresh();

    const path = await defaultPathFresh();
    expect(path).toBe("");

    vi.doUnmock("node:fs/promises");
  });
});
