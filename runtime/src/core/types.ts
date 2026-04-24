/**
 * Shared types for @shift-preflight/runtime.
 */

/** SHIFT drive modes — controls aggressiveness of optimization. */
export type DriveMode = "performance" | "balanced" | "economy";

/** Recognized SHIFT provider names. */
export type ShiftProvider = "anthropic" | "openai" | "google";

/** Per-image optimization metrics. */
export interface ShiftMetrics {
  /** Original image size in bytes. */
  originalBytes: number;
  /** Optimized image size in bytes. */
  optimizedBytes: number;
  /** Bytes saved (originalBytes - optimizedBytes). */
  savedBytes: number;
  /** Estimated tokens saved (provider-specific, may be undefined). */
  savedTokens?: number;
  /** Original MIME type. */
  originalMediaType: string;
  /** Optimized MIME type (may differ if format-converted). */
  optimizedMediaType: string;
  /** SHIFT provider used. */
  provider: string;
  /** Optimization duration in milliseconds. */
  duration: number;
}

/** Input to the optimizer for a single image. */
export interface OptimizeImageInput {
  /** Raw image bytes. */
  buffer: Buffer;
  /** IANA media type (e.g. "image/png"). */
  mediaType: string;
  /** SHIFT provider name. */
  provider: ShiftProvider;
  /** Drive mode. */
  mode: DriveMode;
  /** Optional model name for SHIFT profile lookup. */
  model?: string;
  /** Optional path to shift-ai binary. */
  binary?: string;
}

/** Result from the optimizer for a single image. */
export interface OptimizeImageResult {
  /** Optimized image bytes. */
  buffer: Buffer;
  /** Output MIME type. */
  mediaType: string;
}

/** Marker stored in providerOptions to skip already-optimized images. */
export interface ShiftOptimizedMarker {
  optimized: true;
  originalBytes: number;
  provider: string;
}
