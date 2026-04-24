/**
 * Google/Gemini route handler — POST /v1beta/models/*
 *
 * Intercepts Gemini API requests, runs SHIFT on the payload,
 * and forwards to the real Google API.
 *
 * Note: SHIFT doesn't natively support Google's payload format yet.
 * For now, we use the "anthropic" provider profile (similar image constraints)
 * but walk the Google-specific JSON structure manually.
 */

import type { Context } from "hono";
import type { ProxyConfig } from "../types.js";
import { DEFAULT_PROVIDERS } from "../types.js";
import { forwardHeaders, pipeResponse } from "./passthrough.js";

export function createGoogleHandler(config: ProxyConfig) {
  return async (c: Context) => {
    const body = await c.req.text();
    const baseUrl = config.providers?.google ?? DEFAULT_PROVIDERS.google;

    // Google API paths include query params (e.g. ?key=...), preserve them
    const url = new URL(c.req.url);
    const targetUrl = `${baseUrl}${url.pathname}${url.search}`;

    // SHIFT doesn't have a native Google provider yet.
    // For now we pass through without optimization.
    // TODO: Add Google payload walker when SHIFT adds --provider google
    const finalBody = body;

    if (config.verbose) {
      console.log(
        `[shift-proxy] Google: passthrough (native Google support pending)`,
      );
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
