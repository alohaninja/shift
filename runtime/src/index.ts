/**
 * @shift-ai/runtime — Multimodal preflight for any AI agent.
 *
 * Two integration modes:
 *
 * 1. AI SDK Middleware (in-process, transparent):
 *    ```typescript
 *    import { shiftMiddleware } from "@shift-ai/runtime";
 *    const model = wrapLanguageModel({
 *      model: anthropic("claude-sonnet-4-20250514"),
 *      middleware: shiftMiddleware({ mode: "balanced" }),
 *    });
 *    ```
 *
 * 2. HTTP Proxy (any agent, any language):
 *    ```typescript
 *    import { startProxy } from "@shift-ai/runtime/proxy";
 *    await startProxy({ port: 8787, mode: "balanced" });
 *    ```
 *    Then set: ANTHROPIC_BASE_URL=http://localhost:8787
 *
 * @packageDocumentation
 */

// Middleware (AI SDK)
export { shiftMiddleware } from "./middleware/index.js";
export type { ShiftMiddlewareConfig } from "./middleware/types.js";

// Proxy
export { startProxy, createProxyApp } from "./proxy/index.js";
export type { ProxyConfig } from "./proxy/types.js";

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
