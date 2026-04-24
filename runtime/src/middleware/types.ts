/**
 * Configuration types for the AI SDK middleware.
 */

import type { DriveMode, ShiftMetrics } from "../core/types.js";

export interface ShiftMiddlewareConfig {
  /**
   * SHIFT drive mode — controls aggressiveness of optimization.
   * @default "balanced"
   */
  mode?: DriveMode;

  /**
   * Minimum image size in bytes to trigger optimization.
   * Images smaller than this are passed through unchanged.
   * @default 100_000 (100KB)
   */
  minSize?: number;

  /**
   * Disable optimization entirely (passthrough mode).
   * @default false
   */
  disabled?: boolean;

  /**
   * Override the auto-detected SHIFT provider.
   * Normally inferred from the AI SDK model (e.g. "anthropic", "openai", "google").
   */
  provider?: string;

  /**
   * Override the model name passed to SHIFT for profile lookup.
   * Normally inferred from the AI SDK model ID.
   */
  model?: string;

  /**
   * Path to the shift-ai binary.
   * Auto-detected from PATH if omitted.
   */
  binary?: string;

  /**
   * Callback fired after images are optimized in a request.
   * Receives an array of per-image metrics.
   */
  onOptimize?: (metrics: ShiftMetrics[]) => void;
}
