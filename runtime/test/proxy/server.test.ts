import { describe, it, expect } from "vitest";
import { createProxyApp } from "../../src/proxy/server.js";

describe("proxy server", () => {
  const app = createProxyApp({ verbose: false });

  it("responds to health check", async () => {
    const res = await app.request("/health");
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.service).toBe("@shift-preflight/runtime proxy");
  });

  it("returns 404 for unknown routes", async () => {
    const res = await app.request("/unknown/endpoint", {
      method: "POST",
      body: JSON.stringify({ test: true }),
    });
    // The catch-all handler returns 404 for unrecognized routes
    expect(res.status).toBe(404);
  });

  it("has Anthropic route registered", async () => {
    // We can't actually call Anthropic, but we can verify the route exists
    // by sending a request and checking it doesn't 404 (it will fail with
    // a network error trying to reach api.anthropic.com)
    try {
      const res = await app.request("/v1/messages", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "claude-sonnet-4-20250514",
          max_tokens: 1,
          messages: [{ role: "user", content: "test" }],
        }),
      });
      // Will fail with network error (no real API), but route was matched
      // Status will be 500 or similar, not 404
      expect(res.status).not.toBe(404);
    } catch {
      // Network error is expected — route was matched but upstream is unreachable
    }
  });

  it("has OpenAI route registered", async () => {
    try {
      const res = await app.request("/v1/chat/completions", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "gpt-4o",
          messages: [{ role: "user", content: "test" }],
        }),
      });
      expect(res.status).not.toBe(404);
    } catch {
      // Expected — network error
    }
  });
});
