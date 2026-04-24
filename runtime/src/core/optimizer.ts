/**
 * Core optimizer — calls the shift-ai CLI to optimize individual images.
 *
 * Strategy: wrap the image in a minimal Anthropic-format payload (shift-ai
 * requires a full provider payload), run shift-ai, extract the optimized
 * image back out.
 *
 * For proxy mode, the full request body is passed directly to shift-ai
 * (see proxy/routes/ for that path).
 */

import { execFile } from "node:child_process";
import { writeFile, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { randomBytes } from "node:crypto";

import { isShiftAvailable, getShiftBinary } from "./binary.js";
import type { OptimizeImageInput, OptimizeImageResult } from "./types.js";

const execFileAsync = promisify(execFile);

/**
 * Optimize a single image via the shift-ai CLI.
 *
 * Returns the optimized image, or null if optimization failed or
 * shift-ai is not available.
 */
export async function optimizeImage(
  input: OptimizeImageInput,
): Promise<OptimizeImageResult | null> {
  const available = await isShiftAvailable(input.binary);
  if (!available) return null;

  const bin = input.binary ?? getShiftBinary();
  const id = randomBytes(6).toString("hex");
  const tmpIn = join(tmpdir(), `shift-rt-${id}-in.json`);
  const tmpOut = join(tmpdir(), `shift-rt-${id}-out.json`);

  try {
    // Build a minimal Anthropic-format payload wrapping the single image
    const base64Data = input.buffer.toString("base64");
    const mediaType = toAnthropicMediaType(input.mediaType);

    const wrapperPayload = {
      model: input.model ?? "claude-sonnet-4-20250514",
      max_tokens: 1,
      messages: [
        {
          role: "user",
          content: [
            {
              type: "image",
              source: {
                type: "base64",
                media_type: mediaType,
                data: base64Data,
              },
            },
            { type: "text", text: "." },
          ],
        },
      ],
    };

    await writeFile(tmpIn, JSON.stringify(wrapperPayload));

    // Run shift-ai
    const args = [
      tmpIn,
      "--provider",
      input.provider === "google" ? "anthropic" : input.provider,
      "--mode",
      input.mode,
      "--no-stats",
    ];
    if (input.model) {
      args.push("--model", input.model);
    }

    const { stdout } = await execFileAsync(bin, args, {
      timeout: 30_000,
      maxBuffer: 100 * 1024 * 1024, // 100MB
    });

    // Parse the output payload and extract the optimized image
    const output = JSON.parse(stdout);
    const content = output?.messages?.[0]?.content;
    if (!Array.isArray(content)) return null;

    const imagePart = content.find(
      (p: Record<string, unknown>) => p.type === "image",
    );
    if (!imagePart?.source?.data) return null;

    const optimizedBuffer = Buffer.from(imagePart.source.data, "base64");
    const optimizedMediaType =
      fromAnthropicMediaType(imagePart.source.media_type) ?? input.mediaType;

    // Only return if we actually saved space
    if (optimizedBuffer.length >= input.buffer.length) return null;

    return {
      buffer: optimizedBuffer,
      mediaType: optimizedMediaType,
    };
  } catch {
    // Optimization failed — return null (passthrough)
    return null;
  } finally {
    // Clean up temp files
    await unlink(tmpIn).catch(() => {});
    await unlink(tmpOut).catch(() => {});
  }
}

/**
 * Run shift-ai on a full provider request payload (for proxy mode).
 * Returns the optimized JSON string, or null on failure.
 */
export async function optimizePayload(
  payload: string,
  provider: string,
  mode: string,
  binary?: string,
): Promise<string | null> {
  const available = await isShiftAvailable(binary);
  if (!available) return null;

  const bin = binary ?? getShiftBinary();
  const id = randomBytes(6).toString("hex");
  const tmpIn = join(tmpdir(), `shift-rt-proxy-${id}.json`);

  try {
    await writeFile(tmpIn, payload);

    const args = [tmpIn, "--provider", provider, "--mode", mode, "--no-stats"];

    const { stdout } = await execFileAsync(bin, args, {
      timeout: 60_000,
      maxBuffer: 100 * 1024 * 1024,
    });

    return stdout;
  } catch {
    return null;
  } finally {
    await unlink(tmpIn).catch(() => {});
  }
}

/** Map IANA media types to Anthropic's expected media_type values. */
function toAnthropicMediaType(mediaType: string): string {
  const map: Record<string, string> = {
    "image/jpg": "image/jpeg",
    "image/svg+xml": "image/png", // SHIFT will rasterize
  };
  return map[mediaType] ?? mediaType;
}

/** Map Anthropic media_type back to standard IANA media types. */
function fromAnthropicMediaType(
  anthropicType: string | undefined,
): string | undefined {
  if (!anthropicType) return undefined;
  return anthropicType;
}
