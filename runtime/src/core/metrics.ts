/**
 * Metrics collection for SHIFT optimization runs.
 */

import type { ShiftMetrics } from "./types.js";

/**
 * Create a metrics record for an optimization.
 */
export function createMetrics(opts: {
  originalBytes: number;
  optimizedBytes: number;
  originalMediaType: string;
  optimizedMediaType: string;
  provider: string;
  startTime: number;
}): ShiftMetrics {
  const duration = Date.now() - opts.startTime;
  return {
    originalBytes: opts.originalBytes,
    optimizedBytes: opts.optimizedBytes,
    savedBytes: opts.originalBytes - opts.optimizedBytes,
    originalMediaType: opts.originalMediaType,
    optimizedMediaType: opts.optimizedMediaType,
    provider: opts.provider,
    duration,
  };
}

/**
 * Summarize an array of metrics into aggregate stats.
 */
export function summarizeMetrics(metrics: ShiftMetrics[]): {
  totalOriginalBytes: number;
  totalOptimizedBytes: number;
  totalSavedBytes: number;
  imagesOptimized: number;
  totalDuration: number;
} {
  let totalOriginalBytes = 0;
  let totalOptimizedBytes = 0;
  let totalSavedBytes = 0;
  let totalDuration = 0;

  for (const m of metrics) {
    totalOriginalBytes += m.originalBytes;
    totalOptimizedBytes += m.optimizedBytes;
    totalSavedBytes += m.savedBytes;
    totalDuration += m.duration;
  }

  return {
    totalOriginalBytes,
    totalOptimizedBytes,
    totalSavedBytes,
    imagesOptimized: metrics.length,
    totalDuration,
  };
}
