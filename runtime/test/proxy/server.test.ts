import { describe, it, expect, vi, beforeEach } from "vitest";
import { createProxyApp } from "../../src/proxy/server.js";

// Mock the optimizer so we don't need shift-ai installed
vi.mock("../../src/core/optimizer.js", () => ({
  optimizePayload: vi.fn().mockResolvedValue(null),
}));

// Mock global fetch to avoid real network calls
const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

describe("proxy server", () => {
  const app = createProxyApp({ verbose: false });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("responds to health check", async () => {
    const res = await app.request("/health");
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.service).toBe("@shift-preflight/runtime proxy");
  });

  it("returns 404 for unknown routes via POST", async () => {
    const res = await app.request("/unknown/endpoint", {
      method: "POST",
      body: JSON.stringify({ test: true }),
    });
    expect(res.status).toBe(404);
  });

  describe("Anthropic route", () => {
    it("forwards POST /v1/messages to upstream", async () => {
      mockFetch.mockResolvedValueOnce(
        new Response(JSON.stringify({ id: "msg_123" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );

      const res = await app.request("/v1/messages", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-api-key": "sk-ant-test",
        },
        body: JSON.stringify({
          model: "claude-sonnet-4-20250514",
          max_tokens: 1,
          messages: [{ role: "user", content: "test" }],
        }),
      });

      expect(res.status).toBe(200);
      const body = await res.json();
      expect(body.id).toBe("msg_123");

      // Verify fetch was called with correct upstream URL
      expect(mockFetch).toHaveBeenCalledTimes(1);
      const [url, opts] = mockFetch.mock.calls[0];
      expect(url).toContain("api.anthropic.com");
      expect(url).toContain("/v1/messages");
      expect(opts.method).toBe("POST");
    });

    it("forwards auth headers to upstream", async () => {
      mockFetch.mockResolvedValueOnce(
        new Response("{}", { status: 200 }),
      );

      await app.request("/v1/messages", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-api-key": "sk-ant-test-key",
          "anthropic-version": "2023-06-01",
        },
        body: JSON.stringify({
          model: "claude-sonnet-4-20250514",
          max_tokens: 1,
          messages: [],
        }),
      });

      const [, opts] = mockFetch.mock.calls[0];
      expect(opts.headers["x-api-key"]).toBe("sk-ant-test-key");
      expect(opts.headers["anthropic-version"]).toBe("2023-06-01");
    });

    it("preserves query params in forwarded URL", async () => {
      mockFetch.mockResolvedValueOnce(
        new Response("{}", { status: 200 }),
      );

      await app.request("/v1/messages?beta=true", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ model: "test", max_tokens: 1, messages: [] }),
      });

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain("?beta=true");
    });

    it("returns 502 when upstream is unreachable", async () => {
      mockFetch.mockRejectedValueOnce(new Error("fetch failed: ECONNREFUSED"));

      const res = await app.request("/v1/messages", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "claude-sonnet-4-20250514",
          max_tokens: 1,
          messages: [],
        }),
      });

      expect(res.status).toBe(502);
      const body = await res.json();
      expect(body.error).toBe("Bad Gateway");
    });
  });

  describe("OpenAI route", () => {
    it("forwards POST /v1/chat/completions to upstream", async () => {
      mockFetch.mockResolvedValueOnce(
        new Response(JSON.stringify({ id: "chatcmpl-123" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );

      const res = await app.request("/v1/chat/completions", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: "Bearer sk-test",
        },
        body: JSON.stringify({
          model: "gpt-4o",
          messages: [{ role: "user", content: "test" }],
        }),
      });

      expect(res.status).toBe(200);
      const body = await res.json();
      expect(body.id).toBe("chatcmpl-123");

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain("api.openai.com");
      expect(url).toContain("/v1/chat/completions");
    });

    it("returns 502 when upstream is unreachable", async () => {
      mockFetch.mockRejectedValueOnce(new Error("DNS resolution failed"));

      const res = await app.request("/v1/chat/completions", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ model: "gpt-4o", messages: [] }),
      });

      expect(res.status).toBe(502);
    });
  });

  describe("Google route", () => {
    it("forwards POST /v1beta/models/* to upstream", async () => {
      mockFetch.mockResolvedValueOnce(
        new Response(JSON.stringify({ candidates: [] }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );

      const res = await app.request(
        "/v1beta/models/gemini-2.5-pro:generateContent?key=test-key",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ contents: [] }),
        },
      );

      expect(res.status).toBe(200);

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain("generativelanguage.googleapis.com");
      // Query params preserved
      expect(url).toContain("key=test-key");
    });

    it("returns 502 when upstream is unreachable", async () => {
      mockFetch.mockRejectedValueOnce(new Error("timeout"));

      const res = await app.request(
        "/v1beta/models/gemini-2.5-pro:generateContent",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ contents: [] }),
        },
      );

      expect(res.status).toBe(502);
    });
  });

  describe("passthrough catch-all", () => {
    it("only matches POST (not GET/PUT/DELETE)", async () => {
      // GET to a provider path should not match the catch-all POST handler
      const res = await app.request("/v1/some-other-endpoint", {
        method: "GET",
      });
      // Should get Hono's default 404 since catch-all is now POST-only
      expect(res.status).toBe(404);
    });

    it("returns 502 when upstream is unreachable", async () => {
      mockFetch.mockRejectedValueOnce(new Error("ECONNREFUSED"));

      const res = await app.request("/v1/messages/some-path", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ test: true }),
      });

      expect(res.status).toBe(502);
    });
  });

  describe("custom provider URLs", () => {
    it("uses custom provider URLs when configured", async () => {
      const customApp = createProxyApp({
        providers: {
          anthropic: "https://custom-anthropic.example.com",
        },
      });

      mockFetch.mockResolvedValueOnce(
        new Response("{}", { status: 200 }),
      );

      await customApp.request("/v1/messages", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "claude-sonnet-4-20250514",
          max_tokens: 1,
          messages: [],
        }),
      });

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain("custom-anthropic.example.com");
    });
  });
});
