/**
 * Configuration types for the HTTP proxy.
 */

import type { DriveMode, ShiftMetrics } from "../core/types.js";

export interface ProxyConfig {
  /**
   * Port to listen on.
   * @default 8787
   */
  port?: number;

  /**
   * SHIFT drive mode.
   * @default "balanced"
   */
  mode?: DriveMode;

  /**
   * Minimum image size in bytes to trigger optimization.
   * @default 100_000
   */
  minSize?: number;

  /**
   * Upstream provider base URLs.
   * Defaults to the official API endpoints.
   */
  providers?: {
    anthropic?: string;
    openai?: string;
    google?: string;
  };

  /**
   * Path to the shift-ai binary.
   */
  binary?: string;

  /**
   * Callback fired after images are optimized in a request.
   */
  onOptimize?: (metrics: ShiftMetrics[]) => void;

  /**
   * Enable verbose logging.
   * @default false
   */
  verbose?: boolean;
}

/** Default upstream provider URLs. */
export const DEFAULT_PROVIDERS = {
  anthropic: "https://api.anthropic.com",
  openai: "https://api.openai.com",
  google: "https://generativelanguage.googleapis.com",
} as const;
