import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock child_process and fs before importing the module
vi.mock("node:child_process", () => ({
  execFile: vi.fn(),
}));
vi.mock("node:fs/promises", () => ({
  writeFile: vi.fn(() => Promise.resolve(undefined)),
  unlink: vi.fn(() => Promise.resolve(undefined)),
}));

// Mock binary.ts
vi.mock("../../src/core/binary.js", () => ({
  isShiftAvailable: vi.fn(),
  getShiftBinary: vi.fn().mockReturnValue("shift-ai"),
}));

import { execFile } from "node:child_process";
import { writeFile, unlink } from "node:fs/promises";
import { isShiftAvailable, getShiftBinary } from "../../src/core/binary.js";
import { optimizeImage, optimizePayload } from "../../src/core/optimizer.js";

const mockExecFile = vi.mocked(execFile);
const mockWriteFile = vi.mocked(writeFile);
const mockUnlink = vi.mocked(unlink);
const mockIsAvailable = vi.mocked(isShiftAvailable);

/**
 * Build a fake shift-ai stdout JSON for a single image optimization result.
 */
function fakeShiftOutput(base64Data: string, mediaType = "image/jpeg"): string {
  return JSON.stringify({
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
  });
}

describe("optimizeImage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsAvailable.mockResolvedValue(true);
    // Default: execFile calls back with stdout
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      // Support both (bin, args, opts, cb) and (bin, args, cb) signatures
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "{}", stderr: "" } as never);
      return {} as never;
    });
  });

  it("returns null when shift-ai is not available", async () => {
    mockIsAvailable.mockResolvedValue(false);

    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(result).toBeNull();
    expect(mockExecFile).not.toHaveBeenCalled();
  });

  it("returns null for invalid provider", async () => {
    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "invalid" as never,
      mode: "balanced",
    });

    expect(result).toBeNull();
  });

  it("returns null for invalid mode", async () => {
    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "turbo" as never,
    });

    expect(result).toBeNull();
  });

  it("writes temp file with restricted permissions (0o600)", async () => {
    // Set up shift-ai to return a smaller image
    const smallImage = Buffer.alloc(50_000).toString("base64");
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(smallImage),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(mockWriteFile).toHaveBeenCalledWith(
      expect.stringContaining("shift-rt-"),
      expect.any(String),
      { mode: 0o600 },
    );
  });

  it("returns optimized result when output is smaller", async () => {
    const smallImage = Buffer.alloc(50_000).toString("base64");
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(smallImage, "image/jpeg"),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(result).not.toBeNull();
    expect(result!.buffer.length).toBeLessThan(200_000);
    expect(result!.mediaType).toBe("image/jpeg");
  });

  it("returns null when output is larger than input", async () => {
    const bigImage = Buffer.alloc(300_000).toString("base64");
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(bigImage),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(result).toBeNull();
  });

  it("returns null on malformed JSON output", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, { stdout: "not valid json{", stderr: "" } as never);
      }
      return {} as never;
    });

    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(result).toBeNull();
  });

  it("returns null when execFile throws (timeout, crash)", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(new Error("Process timed out") as never, { stdout: "", stderr: "" } as never);
      }
      return {} as never;
    });

    const result = await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(result).toBeNull();
  });

  it("cleans up temp file on success", async () => {
    const smallImage = Buffer.alloc(50_000).toString("base64");
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(smallImage),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    // Should call unlink for tmpIn only (tmpOut was removed)
    expect(mockUnlink).toHaveBeenCalledTimes(1);
    expect(mockUnlink).toHaveBeenCalledWith(expect.stringContaining("shift-rt-"));
  });

  it("cleans up temp file on error", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(new Error("crash") as never, { stdout: "", stderr: "" } as never);
      }
      return {} as never;
    });

    await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "anthropic",
      mode: "balanced",
    });

    expect(mockUnlink).toHaveBeenCalledTimes(1);
  });

  it("warns when Google provider is used (falls back to Anthropic)", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    const smallImage = Buffer.alloc(50_000).toString("base64");
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(smallImage),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "google",
      mode: "balanced",
    });

    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("Google provider not yet natively supported"),
    );
    warnSpy.mockRestore();
  });

  it("passes --provider anthropic when Google is specified", async () => {
    const smallImage = Buffer.alloc(50_000).toString("base64");
    let capturedArgs: string[] = [];
    mockExecFile.mockImplementation((_bin, args, _opts, cb) => {
      capturedArgs = args as string[];
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: fakeShiftOutput(smallImage),
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    // Suppress the expected Google warning
    vi.spyOn(console, "warn").mockImplementation(() => {});

    await optimizeImage({
      buffer: Buffer.alloc(200_000),
      mediaType: "image/png",
      provider: "google",
      mode: "balanced",
    });

    expect(capturedArgs).toContain("--provider");
    const providerIdx = capturedArgs.indexOf("--provider");
    expect(capturedArgs[providerIdx + 1]).toBe("anthropic");

    vi.restoreAllMocks();
  });
});

describe("optimizePayload", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsAvailable.mockResolvedValue(true);
  });

  it("returns null when shift-ai is not available", async () => {
    mockIsAvailable.mockResolvedValue(false);

    const result = await optimizePayload(
      JSON.stringify({ test: true }),
      "anthropic",
      "balanced",
    );

    expect(result).toBeNull();
  });

  it("returns null for invalid provider", async () => {
    const result = await optimizePayload(
      JSON.stringify({ test: true }),
      "invalid",
      "balanced",
    );

    expect(result).toBeNull();
  });

  it("returns trimmed stdout on success", async () => {
    const optimizedPayload = JSON.stringify({ optimized: true });
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, {
          stdout: optimizedPayload + "\n",
          stderr: "",
        } as never);
      }
      return {} as never;
    });

    const result = await optimizePayload(
      JSON.stringify({ test: true }),
      "anthropic",
      "balanced",
    );

    // Should be trimmed (no trailing newline)
    expect(result).toBe(optimizedPayload);
  });

  it("writes temp file with restricted permissions", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(null, { stdout: "{}", stderr: "" } as never);
      }
      return {} as never;
    });

    await optimizePayload(
      JSON.stringify({ test: true }),
      "anthropic",
      "balanced",
    );

    expect(mockWriteFile).toHaveBeenCalledWith(
      expect.stringContaining("shift-rt-proxy-"),
      expect.any(String),
      { mode: 0o600 },
    );
  });

  it("returns null and logs warning on failure", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) {
        callback(new Error("timeout") as never, { stdout: "", stderr: "" } as never);
      }
      return {} as never;
    });

    const result = await optimizePayload(
      JSON.stringify({ test: true }),
      "anthropic",
      "balanced",
    );

    expect(result).toBeNull();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("optimizePayload failed"),
    );
    warnSpy.mockRestore();
  });
});
