/**
 * @shift-preflight/runtime — Multimodal preflight for any AI agent.
 *
 * AI SDK Middleware (in-process, transparent):
 * ```typescript
 * import { shiftMiddleware } from "@shift-preflight/runtime";
 * const model = wrapLanguageModel({
 *   model: anthropic("claude-sonnet-4-20250514"),
 *   middleware: shiftMiddleware({ mode: "balanced" }),
 * });
 * ```
 *
 * For the HTTP proxy, use `shift-ai proxy start` (native Rust binary).
 * See: https://github.com/alohaninja/shift
 *
 * @packageDocumentation
 */

// Middleware (AI SDK)
export { shiftMiddleware } from "./middleware/index.js";
export type { ShiftMiddlewareConfig } from "./middleware/types.js";

// Core (for advanced usage)
export {
  isShiftAvailable,
  detectProviderFromModel,
  detectProviderFromRoute,
  isOptimizableImage,
  summarizeMetrics,
} from "./core/index.js";
export type {
  DriveMode,
  ShiftProvider,
  ShiftMetrics,
} from "./core/types.js";
