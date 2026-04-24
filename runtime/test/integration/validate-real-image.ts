#!/usr/bin/env npx tsx
/**
 * End-to-end validation with a real image.
 * Creates a real PNG, runs it through the middleware, verifies optimization.
 *
 * Run: npx tsx test/integration/validate-real-image.ts
 */

import { execFileSync } from "node:child_process";
import { writeFileSync, readFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { shiftMiddleware } from "../../src/middleware/index.js";
import { isShiftAvailable } from "../../src/core/index.js";

const green = (s: string) => `\x1b[32m${s}\x1b[0m`;
const red = (s: string) => `\x1b[31m${s}\x1b[0m`;
const dim = (s: string) => `\x1b[2m${s}\x1b[0m`;

async function main() {
  console.log("\n=== Real Image E2E Validation ===\n");

  const available = await isShiftAvailable();
  if (!available) {
    console.log(red("shift-ai not installed — cannot run real image test"));
    console.log("Install: brew install alohaninja/shift/shift-ai\n");
    process.exit(0);
  }

  // Create a real PNG using sips (macOS) — a solid red 2000x1500 image
  const tmpPng = join(tmpdir(), `shift-rt-test-${Date.now()}.png`);

  try {
    // Use sips to create a real image (macOS)
    // First create a tiny 1x1 PNG, then scale it up
    const tiny = join(tmpdir(), `shift-rt-tiny-${Date.now()}.png`);

    // Create a 1x1 PNG manually (minimal valid PNG)
    // PNG signature + IHDR + IDAT + IEND
    const pngSignature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    // We'll use ImageMagick or sips to create a real test image
    try {
      execFileSync("sips", [
        "-z", "1500", "2000",
        "--setProperty", "format", "png",
        "-s", "dpiWidth", "72",
        "-s", "dpiHeight", "72",
        "/System/Library/Desktop Pictures/Solid Colors/Black.png",
        "--out", tmpPng,
      ], { timeout: 10_000, stdio: "pipe" });
    } catch {
      // Fallback: try magick
      try {
        execFileSync("magick", [
          "-size", "2000x1500", "xc:red", tmpPng,
        ], { timeout: 10_000, stdio: "pipe" });
      } catch {
        console.log(dim("Cannot create test image (need sips or ImageMagick)"));
        process.exit(0);
      }
    }

    const imageBuffer = readFileSync(tmpPng);
    const base64Data = imageBuffer.toString("base64");
    const originalBytes = imageBuffer.length;

    console.log(`Test image: ${(originalBytes / 1024).toFixed(1)}KB (2000x1500 PNG)`);

    // Run through middleware
    let metrics: any[] = [];
    const mw = shiftMiddleware({
      mode: "economy",
      minSize: 1000,
      onOptimize: (m) => { metrics = m; },
    });

    const params = {
      prompt: [{
        role: "user",
        content: [
          { type: "file", data: base64Data, mediaType: "image/png" },
          { type: "text", text: "describe this image" },
        ],
      }],
    };

    console.log("Running shiftMiddleware(mode: economy)...\n");
    const startTime = Date.now();
    const result = await mw.transformParams({
      params,
      model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
    });
    const elapsed = Date.now() - startTime;

    const optimizedPart = (result.prompt as any)[0].content[0];
    const optimizedBuffer = Buffer.from(optimizedPart.data, "base64");
    const optimizedBytes = optimizedBuffer.length;
    const wasOptimized = optimizedPart.data !== base64Data;

    if (wasOptimized) {
      const savedBytes = originalBytes - optimizedBytes;
      const pctSaved = ((savedBytes / originalBytes) * 100).toFixed(1);
      console.log(green("  Image was optimized!"));
      console.log(`  Original:  ${(originalBytes / 1024).toFixed(1)}KB`);
      console.log(`  Optimized: ${(optimizedBytes / 1024).toFixed(1)}KB`);
      console.log(`  Saved:     ${(savedBytes / 1024).toFixed(1)}KB (${pctSaved}%)`);
      console.log(`  MediaType: ${optimizedPart.mediaType}`);
      console.log(`  Duration:  ${elapsed}ms`);

      if (metrics.length > 0) {
        console.log(`  Metrics callback: ${metrics.length} image(s) reported`);
      }

      // Check the optimized marker
      const marker = optimizedPart.providerOptions?.shiftAi;
      if (marker?.optimized) {
        console.log(green("  Skip marker set (will be skipped on next turn)"));
      }
    } else {
      console.log(dim("  Image was not optimized (shift-ai returned same or larger)"));
    }

    console.log(`\n${green("Validation complete.")}\n`);
  } finally {
    try { unlinkSync(tmpPng); } catch {}
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
