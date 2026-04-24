/**
 * HTTP Proxy — transparent image optimization for ANY AI agent.
 *
 * Usage:
 *   import { startProxy } from "@shift-ai/runtime/proxy";
 *
 *   const server = await startProxy({ port: 8787, mode: "balanced" });
 *
 * Or via CLI:
 *   npx @shift-ai/runtime proxy --port 8787 --mode balanced
 *
 * Then point your agent's API base URL at http://localhost:8787
 */

import { serve } from "@hono/node-server";
import { createProxyApp } from "./server.js";
import type { ProxyConfig } from "./types.js";
import { isShiftAvailable } from "../core/binary.js";

export type { ProxyConfig } from "./types.js";
export { createProxyApp } from "./server.js";
export { DEFAULT_PROVIDERS } from "./types.js";

/**
 * Start the SHIFT proxy server.
 *
 * Returns the HTTP server instance for graceful shutdown.
 */
export async function startProxy(
  config: ProxyConfig = {},
): Promise<ReturnType<typeof serve>> {
  const port = config.port ?? 8787;
  const app = createProxyApp(config);

  // Check shift-ai availability at startup
  const available = await isShiftAvailable(config.binary);

  const server = serve({ fetch: app.fetch, port });

  console.log(`
┌─────────────────────────────────────────────┐
│  SHIFT Runtime Proxy                        │
│  http://localhost:${String(port).padEnd(5)}                      │
│                                             │
│  Mode: ${(config.mode ?? "balanced").padEnd(12)}                      │
│  SHIFT: ${available ? "available ✓" : "not found (passthrough)"}${available ? "  " : ""}                 │
├─────────────────────────────────────────────┤
│  Route                    → Provider        │
│  POST /v1/messages        → Anthropic       │
│  POST /v1/chat/completions→ OpenAI          │
│  POST /v1beta/models/*    → Google          │
└─────────────────────────────────────────────┘

Set your agent's base URL to http://localhost:${port}
`);

  return server;
}
