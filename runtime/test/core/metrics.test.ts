import { describe, it, expect } from "vitest";
import {
  createMetrics,
  summarizeMetrics,
} from "../../src/core/metrics.js";

describe("createMetrics", () => {
  it("calculates saved bytes and duration", () => {
    const startTime = Date.now() - 100; // 100ms ago
    const metrics = createMetrics({
      originalBytes: 1_000_000,
      optimizedBytes: 200_000,
      originalMediaType: "image/png",
      optimizedMediaType: "image/jpeg",
      provider: "anthropic",
      startTime,
    });

    expect(metrics.originalBytes).toBe(1_000_000);
    expect(metrics.optimizedBytes).toBe(200_000);
    expect(metrics.savedBytes).toBe(800_000);
    expect(metrics.originalMediaType).toBe("image/png");
    expect(metrics.optimizedMediaType).toBe("image/jpeg");
    expect(metrics.provider).toBe("anthropic");
    expect(metrics.duration).toBeGreaterThanOrEqual(90); // ~100ms, some tolerance
    expect(metrics.duration).toBeLessThan(500);
  });
});

describe("summarizeMetrics", () => {
  it("aggregates multiple metrics records", () => {
    const summary = summarizeMetrics([
      {
        originalBytes: 1_000_000,
        optimizedBytes: 200_000,
        savedBytes: 800_000,
        originalMediaType: "image/png",
        optimizedMediaType: "image/jpeg",
        provider: "anthropic",
        duration: 50,
      },
      {
        originalBytes: 500_000,
        optimizedBytes: 100_000,
        savedBytes: 400_000,
        originalMediaType: "image/png",
        optimizedMediaType: "image/jpeg",
        provider: "anthropic",
        duration: 30,
      },
    ]);

    expect(summary.totalOriginalBytes).toBe(1_500_000);
    expect(summary.totalOptimizedBytes).toBe(300_000);
    expect(summary.totalSavedBytes).toBe(1_200_000);
    expect(summary.imagesOptimized).toBe(2);
    expect(summary.totalDuration).toBe(80);
  });

  it("returns zeros for empty array", () => {
    const summary = summarizeMetrics([]);
    expect(summary.totalOriginalBytes).toBe(0);
    expect(summary.imagesOptimized).toBe(0);
  });
});
