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

/** Maximum concurrent shift-ai processes. */
const MAX_CONCURRENT = 8;
let _inflight = 0;
const _waiters: Array<() => void> = [];

async function acquireSlot(): Promise<void> {
  if (_inflight < MAX_CONCURRENT) {
    _inflight++;
    return;
  }
  await new Promise<void>((resolve) => _waiters.push(resolve));
  _inflight++;
}

function releaseSlot(): void {
  _inflight--;
  const next = _waiters.shift();
  if (next) next();
}

const VALID_PROVIDERS = new Set<string>(["anthropic", "openai", "google"]);
const VALID_MODES = new Set<string>(["performance", "balanced", "economy"]);

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

  // Validate provider and mode
  if (!VALID_PROVIDERS.has(input.provider)) {
    console.warn(`[shift-runtime] Invalid provider: ${input.provider}`);
    return null;
  }
  if (!VALID_MODES.has(input.mode)) {
    console.warn(`[shift-runtime] Invalid mode: ${input.mode}`);
    return null;
  }

  const bin = input.binary ?? getShiftBinary();
  const id = randomBytes(6).toString("hex");
  const tmpIn = join(tmpdir(), `shift-rt-${id}-in.json`);

  await acquireSlot();
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

    await writeFile(tmpIn, JSON.stringify(wrapperPayload), { mode: 0o600 });

    // Map Google to Anthropic until shift-ai adds native --provider google
    let cliProvider = input.provider;
    if (cliProvider === "google") {
      console.warn(
        "[shift-runtime] Google provider not yet natively supported by shift-ai; " +
          "using Anthropic optimization profile. Results may not be optimal for Gemini.",
      );
      cliProvider = "anthropic";
    }

    // Run shift-ai
    const args = [
      tmpIn,
      "--provider",
      cliProvider,
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
  } catch (error) {
    // Optimization failed — log and return null (passthrough)
    const msg = error instanceof Error ? error.message : String(error);
    console.warn(`[shift-runtime] optimizeImage failed: ${msg}`);
    return null;
  } finally {
    releaseSlot();
    await unlink(tmpIn).catch(() => {});
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

  // Validate provider and mode
  if (!VALID_PROVIDERS.has(provider)) {
    console.warn(`[shift-runtime] Invalid provider: ${provider}`);
    return null;
  }
  if (!VALID_MODES.has(mode)) {
    console.warn(`[shift-runtime] Invalid mode: ${mode}`);
    return null;
  }

  const bin = binary ?? getShiftBinary();
  const id = randomBytes(6).toString("hex");
  const tmpIn = join(tmpdir(), `shift-rt-proxy-${id}.json`);

  await acquireSlot();
  try {
    await writeFile(tmpIn, payload, { mode: 0o600 });

    const args = [tmpIn, "--provider", provider, "--mode", mode, "--no-stats"];

    const { stdout } = await execFileAsync(bin, args, {
      timeout: 60_000,
      maxBuffer: 100 * 1024 * 1024,
    });

    // Trim trailing whitespace from CLI output
    return stdout.trimEnd();
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    console.warn(`[shift-runtime] optimizePayload failed: ${msg}`);
    return null;
  } finally {
    releaseSlot();
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
