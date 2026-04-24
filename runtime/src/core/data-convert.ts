/**
 * Convert between AI SDK data formats and Node Buffers for SHIFT processing.
 *
 * AI SDK LanguageModelV3FilePart.data can be:
 *   - Uint8Array (binary)
 *   - string (base64 encoded)
 *   - URL (remote reference)
 */

/**
 * Convert AI SDK file part data to a Buffer.
 * Returns the buffer and the original size in bytes.
 */
export async function toBuffer(
  data: Uint8Array | string | URL,
): Promise<{ buffer: Buffer; originalSize: number }> {
  if (data instanceof Uint8Array) {
    const buffer = Buffer.from(data);
    return { buffer, originalSize: buffer.length };
  }

  if (data instanceof URL) {
    const response = await fetch(data.toString());
    if (!response.ok) {
      throw new Error(`Failed to fetch image from ${data}: ${response.status}`);
    }
    const arrayBuffer = await response.arrayBuffer();
    const buffer = Buffer.from(arrayBuffer);
    return { buffer, originalSize: buffer.length };
  }

  // string → base64
  const buffer = Buffer.from(data, "base64");
  return { buffer, originalSize: buffer.length };
}

/**
 * Convert a Buffer back to the same format as the original AI SDK data.
 *
 * If the original was a Uint8Array, returns Uint8Array.
 * If the original was a string (base64), returns base64 string.
 * URLs are converted to base64 strings (can't write back to a URL).
 */
export function fromBuffer(
  buffer: Buffer,
  originalFormat: Uint8Array | string | URL,
): Uint8Array | string {
  if (originalFormat instanceof Uint8Array) {
    return new Uint8Array(buffer);
  }
  // string (base64) or URL → base64 string
  return buffer.toString("base64");
}

/**
 * Check whether a media type is an image type that SHIFT can optimize.
 */
export function isOptimizableImage(mediaType: string): boolean {
  if (!mediaType.startsWith("image/")) return false;
  // SVG is passed through by SHIFT's rasterizer — include it
  // Skip types SHIFT doesn't handle
  const subtype = mediaType.split("/")[1]?.toLowerCase();
  if (!subtype) return false;
  const supported = new Set([
    "png",
    "jpeg",
    "jpg",
    "gif",
    "webp",
    "bmp",
    "tiff",
    "svg+xml",
  ]);
  return supported.has(subtype);
}
