/**
 * Hono HTTP server — the SHIFT proxy.
 *
 * Routes:
 *   POST /v1/messages           → Anthropic (optimize + forward)
 *   POST /v1/chat/completions   → OpenAI (optimize + forward)
 *   POST /v1beta/models/*       → Google (passthrough for now)
 *   POST /*                     → Auto-detect provider + passthrough
 *
 * All auth headers pass through unchanged.
 * SSE/streaming responses are piped directly.
 */

import { Hono } from "hono";
import { createAnthropicHandler } from "./routes/anthropic.js";
import { createOpenAIHandler } from "./routes/openai.js";
import { createGoogleHandler } from "./routes/google.js";
import { createPassthroughHandler } from "./routes/passthrough.js";
import type { ProxyConfig } from "./types.js";

/**
 * Create the Hono application with all proxy routes.
 * This is exported separately from `startProxy` so it can be
 * used in tests or embedded in other servers.
 */
export function createProxyApp(config: ProxyConfig = {}): Hono {
  const app = new Hono();

  // Health check
  app.get("/health", (c) =>
    c.json({ status: "ok", service: "@shift-ai/runtime proxy" }),
  );

  // Anthropic
  app.post("/v1/messages", createAnthropicHandler(config));

  // OpenAI
  app.post("/v1/chat/completions", createOpenAIHandler(config));

  // Google / Gemini
  app.post("/v1beta/models/:model{.+}", createGoogleHandler(config));
  app.post("/v1/models/:model{.+}", createGoogleHandler(config));

  // Catch-all — forward to auto-detected provider
  app.all("/*", createPassthroughHandler(config));

  return app;
}
