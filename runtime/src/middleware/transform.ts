/**
 * The transformParams implementation — walks all messages in the prompt,
 * finds image file parts, and optimizes them via SHIFT.
 */

import {
  detectProviderFromModel,
  toBuffer,
  fromBuffer,
  isOptimizableImage,
  optimizeImage,
  createMetrics,
} from "../core/index.js";
import type { ShiftMetrics, ShiftOptimizedMarker, ShiftProvider } from "../core/types.js";
import type { ShiftMiddlewareConfig } from "./types.js";

/** The marker key used in providerOptions to track already-optimized images. */
const SHIFT_MARKER_KEY = "shiftAi";

/**
 * Transform AI SDK params by optimizing images in all messages.
 *
 * Handles user messages, assistant messages (may contain images),
 * and tool result content (screenshots from tools).
 */
export async function transformPromptImages(
  params: {
    prompt: Array<{
      role: string;
      content: unknown;
      providerOptions?: Record<string, unknown>;
    }>;
    [key: string]: unknown;
  },
  model: { provider?: string; modelId?: string },
  config: ShiftMiddlewareConfig,
): Promise<ShiftMetrics[]> {
  const mode = config.mode ?? "balanced";
  const minSize = config.minSize ?? 100_000;
  const provider = (config.provider ??
    detectProviderFromModel(model) ??
    "anthropic") as ShiftProvider;

  const metrics: ShiftMetrics[] = [];

  for (const message of params.prompt) {
    if (!Array.isArray(message.content)) continue;

    for (let i = 0; i < message.content.length; i++) {
      const part = message.content[i] as Record<string, unknown>;
      if (!part || part.type !== "file") continue;

      const mediaType = part.mediaType as string | undefined;
      if (!mediaType || !isOptimizableImage(mediaType)) continue;

      // Check if already optimized (skip marker)
      const providerOpts = part.providerOptions as
        | Record<string, unknown>
        | undefined;
      const marker = providerOpts?.[SHIFT_MARKER_KEY] as
        | ShiftOptimizedMarker
        | undefined;
      if (marker?.optimized) continue;

      // Convert data to buffer
      const data = part.data as Uint8Array | string | URL;
      if (data === undefined || data === null) continue;

      let bufferResult: { buffer: Buffer; originalSize: number };
      try {
        bufferResult = await toBuffer(data);
      } catch {
        continue; // Can't convert — skip
      }

      // Check minimum size threshold
      if (bufferResult.originalSize < minSize) continue;

      const startTime = Date.now();

      // Optimize via SHIFT
      const result = await optimizeImage({
        buffer: bufferResult.buffer,
        mediaType,
        provider,
        mode,
        model: config.model ?? (model.modelId as string | undefined),
        binary: config.binary,
      });

      if (result) {
        // Replace data in place
        part.data = fromBuffer(result.buffer, data);
        part.mediaType = result.mediaType;

        // Tag as optimized so we skip on subsequent turns
        part.providerOptions = {
          ...(providerOpts ?? {}),
          [SHIFT_MARKER_KEY]: {
            optimized: true,
            originalBytes: bufferResult.originalSize,
            provider,
          } satisfies ShiftOptimizedMarker,
        };

        metrics.push(
          createMetrics({
            originalBytes: bufferResult.originalSize,
            optimizedBytes: result.buffer.length,
            originalMediaType: mediaType,
            optimizedMediaType: result.mediaType,
            provider,
            startTime,
          }),
        );
      }
    }

    // Also handle tool result content parts (screenshots from tools)
    if (message.role === "tool" && Array.isArray(message.content)) {
      for (const toolResultPart of message.content as Array<
        Record<string, unknown>
      >) {
        if (toolResultPart.type !== "tool-result") continue;
        const output = toolResultPart.output as Record<string, unknown> | undefined;
        if (!output || output.type !== "content") continue;

        const contentParts = output.value as
          | Array<Record<string, unknown>>
          | undefined;
        if (!Array.isArray(contentParts)) continue;

        for (const contentPart of contentParts) {
          // Handle image-data and file-data types in tool results
          if (
            contentPart.type !== "image-data" &&
            contentPart.type !== "file-data"
          )
            continue;

          const partMediaType = contentPart.mediaType as string | undefined;
          if (!partMediaType || !isOptimizableImage(partMediaType)) continue;

          const partData = contentPart.data as string | undefined;
          if (!partData) continue;

          const buffer = Buffer.from(partData, "base64");
          if (buffer.length < minSize) continue;

          const startTime = Date.now();
          const result = await optimizeImage({
            buffer,
            mediaType: partMediaType,
            provider,
            mode,
            model: config.model ?? (model.modelId as string | undefined),
            binary: config.binary,
          });

          if (result) {
            contentPart.data = result.buffer.toString("base64");
            contentPart.mediaType = result.mediaType;

            metrics.push(
              createMetrics({
                originalBytes: buffer.length,
                optimizedBytes: result.buffer.length,
                originalMediaType: partMediaType,
                optimizedMediaType: result.mediaType,
                provider,
                startTime,
              }),
            );
          }
        }
      }
    }
  }

  return metrics;
}
