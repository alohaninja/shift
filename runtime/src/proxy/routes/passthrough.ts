/**
 * Passthrough utilities and catch-all handler.
 *
 * Forwards requests unchanged to the detected upstream provider.
 * Also exports shared helpers for header forwarding and response piping.
 */

import type { Context } from "hono";
import { detectProviderFromRoute } from "../../core/provider-detect.js";
import type { ProxyConfig } from "../types.js";
import { DEFAULT_PROVIDERS } from "../types.js";

/**
 * Forward request headers, stripping specified headers.
 * Auth headers (Authorization, x-api-key, x-goog-api-key) pass through.
 */
export function forwardHeaders(
  original: Headers,
  stripHeaders: string[] = [],
): Record<string, string> {
  const result: Record<string, string> = {};
  const strip = new Set(stripHeaders.map((h) => h.toLowerCase()));

  original.forEach((value, key) => {
    if (!strip.has(key.toLowerCase())) {
      result[key] = value;
    }
  });

  return result;
}

/**
 * Headers that must be stripped from upstream responses.
 *
 * Node's fetch() automatically decompresses response bodies, so the
 * upstream content-encoding/content-length headers are stale by the time
 * we forward the response. Passing them through causes clients to attempt
 * double-decompression (e.g. "Decompression error: ZlibError" in Claude Code).
 */
const STRIP_RESPONSE_HEADERS = new Set([
  "content-encoding",
  "content-length",
  "transfer-encoding",
]);

/**
 * Pipe a fetch Response back through Hono.
 * Streams SSE/chunked responses directly without buffering.
 */
export function pipeResponse(_c: Context, response: Response): Response {
  const headers: Record<string, string> = {};
  response.headers.forEach((value, key) => {
    if (!STRIP_RESPONSE_HEADERS.has(key.toLowerCase())) {
      headers[key] = value;
    }
  });

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

/**
 * Catch-all handler: forwards requests unchanged to the upstream provider
 * detected from the route path.
 */
export function createPassthroughHandler(config: ProxyConfig) {
  return async (c: Context) => {
    const path = c.req.path;
    const provider = detectProviderFromRoute(path);

    if (!provider) {
      return c.json(
        { error: "Unknown route — cannot determine upstream provider" },
        404,
      );
    }

    const baseUrl =
      config.providers?.[provider] ??
      DEFAULT_PROVIDERS[provider];

    const url = new URL(c.req.url);
    const targetUrl = `${baseUrl}${url.pathname}${url.search}`;

    if (config.verbose) {
      // Redact query params from log output (may contain API keys)
      console.log(`[shift-proxy] Passthrough: ${path} → ${baseUrl}${url.pathname}`);
    }

    const hasBody = !["GET", "HEAD"].includes(c.req.method.toUpperCase());
    const body = hasBody ? await c.req.text() : undefined;
    const headers = forwardHeaders(c.req.raw.headers, [
      "host",
      "content-length",
    ]);

    try {
      const response = await fetch(targetUrl, {
        method: c.req.method,
        headers,
        body: body || undefined,
        signal: AbortSignal.timeout(120_000),
      });

      return pipeResponse(c, response);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      console.error(`[shift-proxy] Passthrough upstream error: ${msg}`);
      return c.json({ error: "Bad Gateway", detail: "Upstream provider unreachable" }, 502);
    }
  };
}
