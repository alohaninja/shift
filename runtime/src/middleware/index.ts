/**
 * AI SDK Middleware — transparent image optimization for any AI SDK app.
 *
 * Usage:
 *   import { shiftMiddleware } from "@shift-ai/runtime";
 *   import { wrapLanguageModel } from "ai";
 *
 *   const model = wrapLanguageModel({
 *     model: anthropic("claude-sonnet-4-20250514"),
 *     middleware: shiftMiddleware({ mode: "balanced" }),
 *   });
 */

import { transformPromptImages } from "./transform.js";
import type { ShiftMiddlewareConfig } from "./types.js";

export type { ShiftMiddlewareConfig } from "./types.js";

/**
 * Create a SHIFT middleware for the Vercel AI SDK.
 *
 * Intercepts all messages in `transformParams` and optimizes images
 * via shift-ai before they reach the LLM provider.
 *
 * The middleware type is `LanguageModelV3Middleware` from `@ai-sdk/provider`,
 * but we define the shape inline to keep `@ai-sdk/provider` as an optional
 * peer dependency.
 */
export function shiftMiddleware(
  config: ShiftMiddlewareConfig = {},
): {
  transformParams: (opts: {
    params: Record<string, unknown>;
    model: { provider?: string; modelId?: string };
  }) => Promise<Record<string, unknown>>;
} {
  return {
    transformParams: async ({ params, model }) => {
      if (config.disabled) return params;

      const prompt = params.prompt as
        | Array<{
            role: string;
            content: unknown;
            providerOptions?: Record<string, unknown>;
          }>
        | undefined;

      if (!Array.isArray(prompt)) return params;

      const metrics = await transformPromptImages(
        { ...params, prompt },
        model,
        config,
      );

      if (metrics.length > 0) {
        config.onOptimize?.(metrics);
      }

      return params;
    },
  };
}
