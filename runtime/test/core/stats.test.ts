import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { readFile, unlink, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes } from "node:crypto";
import {
  buildRunRecord,
  recordRun,
  getSessionStats,
  resetSessionStats,
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

  it("does not throw on write failure (swallows errors)", async () => {
    // Point at a path that's a directory (can't write to a directory)
    const dirPath = join(tmpdir(), `shift-test-dir-${randomBytes(4).toString("hex")}`);
    await mkdir(dirPath, { recursive: true });

    const record = buildRunRecord({
      provider: "anthropic",
      originalBytes: 50_000,
      optimizedBytes: 20_000,
      durationMs: 50,
    });

    // Should not throw
    await expect(recordRun(record, dirPath)).resolves.toBeUndefined();
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
});
