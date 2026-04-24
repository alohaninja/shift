/**
 * Core module — shared utilities used by both middleware and proxy.
 */

export { isShiftAvailable, getShiftBinary, _resetBinaryCache } from "./binary.js";
export { detectProviderFromModel, detectProviderFromRoute } from "./provider-detect.js";
export { toBuffer, fromBuffer, isOptimizableImage } from "./data-convert.js";
export { optimizeImage, optimizePayload } from "./optimizer.js";
export { createMetrics, summarizeMetrics } from "./metrics.js";
export type {
  DriveMode,
  ShiftProvider,
  ShiftMetrics,
  OptimizeImageInput,
  OptimizeImageResult,
  ShiftOptimizedMarker,
} from "./types.js";
