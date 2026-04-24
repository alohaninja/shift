import { describe, it, expect } from "vitest";
import {
  toBuffer,
  fromBuffer,
  isOptimizableImage,
} from "../../src/core/data-convert.js";

describe("toBuffer", () => {
  it("converts Uint8Array to Buffer", async () => {
    const input = new Uint8Array([0x89, 0x50, 0x4e, 0x47]); // PNG magic bytes
    const { buffer, originalSize } = await toBuffer(input);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toBe(4);
    expect(originalSize).toBe(4);
    expect(buffer[0]).toBe(0x89);
  });

  it("converts base64 string to Buffer", async () => {
    const original = Buffer.from("hello world");
    const base64 = original.toString("base64");
    const { buffer, originalSize } = await toBuffer(base64);
    expect(buffer.toString()).toBe("hello world");
    expect(originalSize).toBe(11);
  });

  it("preserves binary data through round-trip", async () => {
    const original = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]);
    const base64 = original.toString("base64");
    const { buffer } = await toBuffer(base64);
    expect(Buffer.compare(buffer, original)).toBe(0);
  });
});

describe("fromBuffer", () => {
  it("returns Uint8Array when original was Uint8Array", () => {
    const buffer = Buffer.from([1, 2, 3]);
    const original = new Uint8Array([0]);
    const result = fromBuffer(buffer, original);
    expect(result).toBeInstanceOf(Uint8Array);
    expect((result as Uint8Array).length).toBe(3);
  });

  it("returns base64 string when original was string", () => {
    const buffer = Buffer.from("test data");
    const result = fromBuffer(buffer, "originalBase64");
    expect(typeof result).toBe("string");
    expect(Buffer.from(result as string, "base64").toString()).toBe("test data");
  });

  it("returns base64 string when original was URL", () => {
    const buffer = Buffer.from("test data");
    const result = fromBuffer(buffer, new URL("https://example.com/img.png"));
    expect(typeof result).toBe("string");
  });
});

describe("isOptimizableImage", () => {
  it("returns true for image/png", () => {
    expect(isOptimizableImage("image/png")).toBe(true);
  });

  it("returns true for image/jpeg", () => {
    expect(isOptimizableImage("image/jpeg")).toBe(true);
  });

  it("returns true for image/gif", () => {
    expect(isOptimizableImage("image/gif")).toBe(true);
  });

  it("returns true for image/webp", () => {
    expect(isOptimizableImage("image/webp")).toBe(true);
  });

  it("returns true for image/svg+xml", () => {
    expect(isOptimizableImage("image/svg+xml")).toBe(true);
  });

  it("returns true for image/bmp", () => {
    expect(isOptimizableImage("image/bmp")).toBe(true);
  });

  it("returns false for text/plain", () => {
    expect(isOptimizableImage("text/plain")).toBe(false);
  });

  it("returns false for application/pdf", () => {
    expect(isOptimizableImage("application/pdf")).toBe(false);
  });

  it("returns false for audio/mp3", () => {
    expect(isOptimizableImage("audio/mp3")).toBe(false);
  });

  it("returns false for image/x-custom", () => {
    expect(isOptimizableImage("image/x-custom")).toBe(false);
  });
});
