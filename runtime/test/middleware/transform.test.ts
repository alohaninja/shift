import { describe, it, expect, vi, beforeEach } from "vitest";
import { shiftMiddleware } from "../../src/middleware/index.js";

// Mock the optimizer to test the middleware's handling of optimization results
vi.mock("../../src/core/optimizer.js", () => ({
  optimizeImage: vi.fn().mockResolvedValue(null),
}));

// Mock binary check to always return true
vi.mock("../../src/core/binary.js", () => ({
  isShiftAvailable: vi.fn().mockResolvedValue(true),
  getShiftBinary: vi.fn().mockReturnValue("shift-ai"),
}));

import { optimizeImage } from "../../src/core/optimizer.js";
const mockOptimizeImage = vi.mocked(optimizeImage);

describe("shiftMiddleware", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockOptimizeImage.mockResolvedValue(null);
  });

  it("returns a middleware with transformParams", () => {
    const middleware = shiftMiddleware();
    expect(middleware).toHaveProperty("transformParams");
    expect(typeof middleware.transformParams).toBe("function");
  });

  it("passes through when disabled", async () => {
    const middleware = shiftMiddleware({ disabled: true });
    const params = {
      prompt: [
        {
          role: "user",
          content: [
            { type: "text", text: "hello" },
            {
              type: "file",
              data: "base64data",
              mediaType: "image/png",
            },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
    });

    expect(result).toBe(params);
  });

  it("passes through when prompt is not an array", async () => {
    const middleware = shiftMiddleware();
    const params = { prompt: "just a string" };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    expect(result).toBe(params);
  });

  it("passes through text-only messages unchanged", async () => {
    const middleware = shiftMiddleware();
    const params = {
      prompt: [
        {
          role: "user",
          content: [{ type: "text", text: "hello" }],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    // Should return same object (no mutation needed)
    expect(result).toBe(params);
  });

  it("skips images below minSize threshold", async () => {
    const middleware = shiftMiddleware({ minSize: 1_000_000 }); // 1MB
    const smallImage = Buffer.alloc(100).toString("base64"); // 100 bytes

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file",
              data: smallImage,
              mediaType: "image/png",
            },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    // Image should be untouched (below threshold)
    const filePart = (result.prompt as Array<{ content: Array<Record<string, unknown>> }>)[0]?.content[0];
    expect(filePart?.data).toBe(smallImage);
    // optimizeImage should not have been called
    expect(mockOptimizeImage).not.toHaveBeenCalled();
  });

  it("skips non-image file types", async () => {
    const middleware = shiftMiddleware();
    const pdfData = Buffer.alloc(200_000).toString("base64");

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file",
              data: pdfData,
              mediaType: "application/pdf",
            },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    const filePart = (result.prompt as Array<{ content: Array<Record<string, unknown>> }>)[0]?.content[0];
    expect(filePart?.data).toBe(pdfData);
    expect(mockOptimizeImage).not.toHaveBeenCalled();
  });

  it("skips images already marked as optimized", async () => {
    const middleware = shiftMiddleware();
    const imageData = Buffer.alloc(200_000).toString("base64");

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file",
              data: imageData,
              mediaType: "image/png",
              providerOptions: {
                shiftAi: {
                  optimized: true,
                  originalBytes: 500_000,
                  provider: "anthropic",
                },
              },
            },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    // Should be untouched
    const filePart = (result.prompt as Array<{ content: Array<Record<string, unknown>> }>)[0]?.content[0];
    expect(filePart?.data).toBe(imageData);
    expect(mockOptimizeImage).not.toHaveBeenCalled();
  });

  it("replaces image data when optimization succeeds", async () => {
    const optimizedBuffer = Buffer.alloc(50_000); // smaller
    mockOptimizeImage.mockResolvedValueOnce({
      buffer: optimizedBuffer,
      mediaType: "image/jpeg",
    });

    const originalImage = Buffer.alloc(200_000).toString("base64");
    const middleware = shiftMiddleware({ minSize: 0 }); // no min size

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file" as const,
              data: originalImage,
              mediaType: "image/png",
            },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
    });

    expect(mockOptimizeImage).toHaveBeenCalledTimes(1);

    const filePart = (result.prompt as Array<{ content: Array<Record<string, unknown>> }>)[0]?.content[0];
    // Data should be replaced with base64-encoded optimized buffer
    expect(filePart?.data).toBe(optimizedBuffer.toString("base64"));
    // Media type should be updated
    expect(filePart?.mediaType).toBe("image/jpeg");
    // Should have optimization marker
    const providerOpts = filePart?.providerOptions as Record<string, Record<string, unknown>>;
    expect(providerOpts?.shiftAi?.optimized).toBe(true);
    expect(providerOpts?.shiftAi?.originalBytes).toBeGreaterThan(0);
  });

  it("calls onOptimize callback with metrics when optimization occurs", async () => {
    const optimizedBuffer = Buffer.alloc(50_000);
    mockOptimizeImage.mockResolvedValueOnce({
      buffer: optimizedBuffer,
      mediaType: "image/jpeg",
    });

    const onOptimize = vi.fn();
    const middleware = shiftMiddleware({ onOptimize, minSize: 0 });

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file" as const,
              data: Buffer.alloc(200_000).toString("base64"),
              mediaType: "image/png",
            },
          ],
        },
      ],
    };

    await middleware.transformParams({
      params,
      model: { provider: "anthropic", modelId: "claude-sonnet-4-20250514" },
    });

    expect(onOptimize).toHaveBeenCalledTimes(1);
    const metrics = onOptimize.mock.calls[0][0];
    expect(metrics).toHaveLength(1);
    expect(metrics[0].savedBytes).toBeGreaterThan(0);
    expect(metrics[0].provider).toBe("anthropic");
  });

  it("does not call onOptimize when no optimization happens", async () => {
    const onOptimize = vi.fn();
    const middleware = shiftMiddleware({ onOptimize, minSize: 1_000_000 });

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            {
              type: "file",
              data: Buffer.alloc(100).toString("base64"),
              mediaType: "image/png",
            },
          ],
        },
      ],
    };

    await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    // No images above threshold, so callback should not fire
    expect(onOptimize).not.toHaveBeenCalled();
  });

  it("handles multiple images in a single message", async () => {
    const optimizedBuffer = Buffer.alloc(50_000);
    mockOptimizeImage.mockResolvedValue({
      buffer: optimizedBuffer,
      mediaType: "image/jpeg",
    });

    const middleware = shiftMiddleware({ minSize: 0 });
    const originalImage = Buffer.alloc(200_000).toString("base64");

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            { type: "file", data: originalImage, mediaType: "image/png" },
            { type: "text", text: "describe these" },
            { type: "file", data: originalImage, mediaType: "image/png" },
          ],
        },
      ],
    };

    await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    expect(mockOptimizeImage).toHaveBeenCalledTimes(2);
  });

  it("leaves image unchanged when optimizer returns null", async () => {
    mockOptimizeImage.mockResolvedValue(null);

    const originalImage = Buffer.alloc(200_000).toString("base64");
    const middleware = shiftMiddleware({ minSize: 0 });

    const params = {
      prompt: [
        {
          role: "user",
          content: [
            { type: "file", data: originalImage, mediaType: "image/png" },
          ],
        },
      ],
    };

    const result = await middleware.transformParams({
      params,
      model: { provider: "anthropic" },
    });

    const filePart = (result.prompt as Array<{ content: Array<Record<string, unknown>> }>)[0]?.content[0];
    // Should remain unchanged
    expect(filePart?.data).toBe(originalImage);
    expect(filePart?.mediaType).toBe("image/png");
    // Should NOT have optimization marker
    expect(filePart?.providerOptions).toBeUndefined();
  });
});
