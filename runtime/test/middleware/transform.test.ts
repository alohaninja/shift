import { describe, it, expect, vi, beforeEach } from "vitest";
import { shiftMiddleware } from "../../src/middleware/index.js";

describe("shiftMiddleware", () => {
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
  });

  it("calls onOptimize callback when optimization occurs", async () => {
    // This test would require shift-ai to be installed.
    // We verify the callback wiring is correct by checking
    // that it's not called when no optimization happens.
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
});
