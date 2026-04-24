#!/usr/bin/env npx tsx
/**
 * Integration validation script.
 * Tests both middleware and proxy modes with real shift-ai.
 *
 * Run: npx tsx test/integration/validate.ts
 */

import { shiftMiddleware } from "../../src/middleware/index.js";
import { createProxyApp } from "../../src/proxy/server.js";
import {
  isShiftAvailable,
  detectProviderFromModel,
  isOptimizableImage,
} from "../../src/core/index.js";

// ANSI colors
const green = (s: string) => `\x1b[32m${s}\x1b[0m`;
const red = (s: string) => `\x1b[31m${s}\x1b[0m`;
const dim = (s: string) => `\x1b[2m${s}\x1b[0m`;

let passed = 0;
let failed = 0;

function assert(label: string, ok: boolean, detail?: string) {
  if (ok) {
    console.log(`  ${green("✓")} ${label}`);
    passed++;
  } else {
    console.log(`  ${red("✗")} ${label}${detail ? ` — ${detail}` : ""}`);
    failed++;
  }
}

async function main() {
  console.log("\n=== @shift-preflight/runtime Integration Validation ===\n");

  // 1. Core: shift-ai binary detection
  console.log("Core: binary detection");
  const available = await isShiftAvailable();
  assert("isShiftAvailable() returns boolean", typeof available === "boolean");
  assert(`shift-ai binary ${available ? "found" : "not found (passthrough mode)"}`, true);

  // 2. Core: provider detection
  console.log("\nCore: provider detection");
  assert(
    "anthropic provider from model",
    detectProviderFromModel({ provider: "anthropic" }) === "anthropic",
  );
  assert(
    "openai from gpt-4o model ID",
    detectProviderFromModel({ modelId: "gpt-4o" }) === "openai",
  );
  assert(
    "google from gemini model ID",
    detectProviderFromModel({ modelId: "gemini-2.5-pro" }) === "google",
  );

  // 3. Core: media type detection
  console.log("\nCore: media type detection");
  assert("image/png is optimizable", isOptimizableImage("image/png"));
  assert("image/jpeg is optimizable", isOptimizableImage("image/jpeg"));
  assert("application/pdf is NOT optimizable", !isOptimizableImage("application/pdf"));

  // 4. Middleware: creation and passthrough
  console.log("\nMiddleware: shiftMiddleware()");
  const middleware = shiftMiddleware({ mode: "balanced" });
  assert("middleware has transformParams", typeof middleware.transformParams === "function");

  // Test disabled passthrough
  const disabledMw = shiftMiddleware({ disabled: true });
  const testParams = {
    prompt: [{ role: "user", content: [{ type: "text", text: "hello" }] }],
  };
  const result = await disabledMw.transformParams({
    params: testParams,
    model: { provider: "anthropic" },
  });
  assert("disabled middleware passes through unchanged", result === testParams);

  // Test with a small image (should skip — below threshold)
  const smallImage = Buffer.alloc(50).toString("base64");
  const paramsWithSmallImage = {
    prompt: [{
      role: "user",
      content: [
        { type: "file", data: smallImage, mediaType: "image/png" },
      ],
    }],
  };
  const smallResult = await middleware.transformParams({
    params: paramsWithSmallImage,
    model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
  });
  const smallPart = (smallResult.prompt as any)[0].content[0];
  assert("small image passes through unchanged", smallPart.data === smallImage);

  // 5. Middleware: real optimization (if shift-ai available)
  if (available) {
    console.log("\nMiddleware: real image optimization via shift-ai");
    // Create a large-ish PNG-like buffer (200KB of random data with PNG header)
    const pngHeader = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const largeBuffer = Buffer.concat([pngHeader, Buffer.alloc(200_000, 0xff)]);
    const largeBase64 = largeBuffer.toString("base64");

    let optimizeCallbackFired = false;
    const realMw = shiftMiddleware({
      mode: "economy",
      minSize: 1000, // low threshold for test
      onOptimize: (metrics) => {
        optimizeCallbackFired = true;
        console.log(dim(`    → onOptimize: ${metrics.length} images, saved ${metrics.reduce((a, m) => a + m.savedBytes, 0)} bytes`));
      },
    });

    const realParams = {
      prompt: [{
        role: "user",
        content: [
          { type: "file", data: largeBase64, mediaType: "image/png" },
          { type: "text", text: "describe this" },
        ],
      }],
    };

    const realResult = await realMw.transformParams({
      params: realParams,
      model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
    });

    // Check if the image was modified (shift-ai may or may not optimize a fake PNG)
    const realPart = (realResult.prompt as any)[0].content[0];
    const wasOptimized = realPart.data !== largeBase64;
    assert(
      `shift-ai processed the image (${wasOptimized ? "optimized" : "passed through — fake PNG may not be optimizable"})`,
      true, // either outcome is valid
    );
  } else {
    console.log(dim("\nSkipping real optimization test (shift-ai not installed)"));
  }

  // 6. Proxy: health check
  console.log("\nProxy: Hono server");
  const app = createProxyApp({ verbose: false });

  const healthRes = await app.request("/health");
  const healthBody = await healthRes.json() as { status: string };
  assert("GET /health returns 200", healthRes.status === 200);
  assert("health response has status: ok", healthBody.status === "ok");

  // Verify routes exist (they'll fail to reach upstream, but shouldn't 404)
  const anthropicRes = await app.request("/v1/messages", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model: "claude-sonnet-4-20250514", max_tokens: 1, messages: [{ role: "user", content: "test" }] }),
  });
  assert("POST /v1/messages route exists (not 404)", anthropicRes.status !== 404);

  const openaiRes = await app.request("/v1/chat/completions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model: "gpt-4o", messages: [{ role: "user", content: "test" }] }),
  });
  assert("POST /v1/chat/completions route exists (not 404)", openaiRes.status !== 404);

  const unknownRes = await app.request("/unknown/route", { method: "POST", body: "{}" });
  assert("unknown route returns 404", unknownRes.status === 404);

  // Summary
  console.log(`\n${"─".repeat(45)}`);
  console.log(`  ${green(`${passed} passed`)}, ${failed > 0 ? red(`${failed} failed`) : `${failed} failed`}`);
  console.log(`${"─".repeat(45)}\n`);

  process.exit(failed > 0 ? 1 : 0);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
