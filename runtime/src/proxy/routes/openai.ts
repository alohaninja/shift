/**
 * OpenAI route handler — POST /v1/chat/completions
 *
 * Intercepts OpenAI API requests, runs SHIFT on the payload,
 * and forwards to the real OpenAI API.
 */

import type { Context } from "hono";
import { optimizePayload } from "../../core/optimizer.js";
import type { ProxyConfig } from "../types.js";
import { DEFAULT_PROVIDERS } from "../types.js";
import { forwardHeaders, pipeResponse } from "./passthrough.js";

export function createOpenAIHandler(config: ProxyConfig) {
  return async (c: Context) => {
    const body = await c.req.text();
    const mode = config.mode ?? "balanced";
    const baseUrl = config.providers?.openai ?? DEFAULT_PROVIDERS.openai;
    const targetUrl = `${baseUrl}${c.req.path}`;

    // Optimize the payload via shift-ai
    const optimized = await optimizePayload(body, "openai", mode, config.binary);
    const finalBody = optimized ?? body;

    if (config.verbose && optimized) {
      const savedBytes = Buffer.byteLength(body) - Buffer.byteLength(finalBody);
      if (savedBytes > 0) {
        console.log(
          `[shift-proxy] OpenAI: saved ${(savedBytes / 1024).toFixed(1)}KB`,
        );
      }
    }

    const headers = forwardHeaders(c.req.raw.headers, [
      "host",
      "content-length",
    ]);

    const response = await fetch(targetUrl, {
      method: "POST",
      headers,
      body: finalBody,
    });

    return pipeResponse(c, response);
  };
}
