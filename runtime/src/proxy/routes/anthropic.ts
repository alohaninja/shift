/**
 * Anthropic route handler — POST /v1/messages
 *
 * Intercepts Anthropic API requests, runs SHIFT on the payload,
 * and forwards to the real Anthropic API.
 */

import type { Context } from "hono";
import { optimizePayload } from "../../core/optimizer.js";
import { buildRunRecord, recordRun } from "../../core/stats.js";
import type { ProxyConfig } from "../types.js";
import { DEFAULT_PROVIDERS } from "../types.js";
import { forwardHeaders, pipeResponse } from "./passthrough.js";

export function createAnthropicHandler(config: ProxyConfig) {
  return async (c: Context) => {
    const body = await c.req.text();
    const mode = config.mode ?? "balanced";
    const baseUrl =
      config.providers?.anthropic ?? DEFAULT_PROVIDERS.anthropic;
    const url = new URL(c.req.url);
    const targetUrl = `${baseUrl}${url.pathname}${url.search}`;

    // Optimize the payload via shift-ai
    const startTime = Date.now();
    const optimized = await optimizePayload(body, "anthropic", mode, config.binary);
    const durationMs = Date.now() - startTime;
    const finalBody = optimized ?? body;

    const originalBytes = Buffer.byteLength(body);
    const optimizedBytes = Buffer.byteLength(finalBody);

    if (config.verbose && optimized) {
      const savedBytes = originalBytes - optimizedBytes;
      if (savedBytes > 0) {
        console.log(
          `[shift-proxy] Anthropic: saved ${(savedBytes / 1024).toFixed(1)}KB`,
        );
      }
    }

    // Record stats (fire-and-forget — never blocks the request)
    if (optimized) {
      const record = buildRunRecord({
        provider: "anthropic",
        originalBytes,
        optimizedBytes,
        durationMs,
        source: "proxy",
      });
      recordRun(record).catch((e) => {
        // Already logged inside recordRun; outer catch prevents unhandled rejection
      });
    }

    // Forward to the real API
    const headers = forwardHeaders(c.req.raw.headers, [
      "host",
      "content-length",
    ]);

    try {
      const response = await fetch(targetUrl, {
        method: "POST",
        headers,
        body: finalBody,
        signal: AbortSignal.timeout(120_000),
      });

      return pipeResponse(response);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      console.error(`[shift-proxy] Anthropic upstream error: ${msg}`);
      return c.json({ error: "Bad Gateway", detail: "Upstream provider unreachable" }, 502);
    }
  };
}
