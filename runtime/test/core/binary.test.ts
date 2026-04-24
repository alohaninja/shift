import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  isShiftAvailable,
  getShiftBinary,
  _resetBinaryCache,
} from "../../src/core/binary.js";

// Mock child_process
vi.mock("node:child_process", () => ({
  execFile: vi.fn(),
}));

import { execFile } from "node:child_process";

const mockExecFile = vi.mocked(execFile);

describe("binary detection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    _resetBinaryCache();
  });

  it("returns true when shift-ai is found", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "0.4.1\n", stderr: "" } as never);
      return {} as never;
    });

    const result = await isShiftAvailable();
    expect(result).toBe(true);
    expect(getShiftBinary()).toBe("shift-ai");
  });

  it("returns false when shift-ai is not found", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(new Error("ENOENT") as never, { stdout: "", stderr: "" } as never);
      return {} as never;
    });

    const result = await isShiftAvailable();
    expect(result).toBe(false);
    warnSpy.mockRestore();
  });

  it("caches result for same binary path", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "0.4.1\n", stderr: "" } as never);
      return {} as never;
    });

    await isShiftAvailable();
    await isShiftAvailable();

    // Should only call execFile once (cached)
    expect(mockExecFile).toHaveBeenCalledTimes(1);
  });

  it("caches separately for different binary paths", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "0.4.1\n", stderr: "" } as never);
      return {} as never;
    });

    await isShiftAvailable("/custom/shift-ai");
    await isShiftAvailable(); // default path

    // Should call execFile twice (different paths)
    expect(mockExecFile).toHaveBeenCalledTimes(2);
  });

  it("does not re-check same custom path", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "0.4.1\n", stderr: "" } as never);
      return {} as never;
    });

    await isShiftAvailable("/custom/shift-ai");
    await isShiftAvailable("/custom/shift-ai");

    // Should only call execFile once for the custom path
    expect(mockExecFile).toHaveBeenCalledTimes(1);
  });

  it("warns only once across multiple failures (same cache session)", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(new Error("ENOENT") as never, { stdout: "", stderr: "" } as never);
      return {} as never;
    });

    // Both calls fail, but warning should only fire once
    await isShiftAvailable();
    await isShiftAvailable("/other/path");

    expect(warnSpy).toHaveBeenCalledTimes(1);
    warnSpy.mockRestore();
  });

  it("updates getShiftBinary when custom path succeeds", async () => {
    mockExecFile.mockImplementation((_bin, _args, _opts, cb) => {
      const callback = typeof _opts === "function" ? _opts : cb;
      if (callback) callback(null, { stdout: "0.4.1\n", stderr: "" } as never);
      return {} as never;
    });

    await isShiftAvailable("/custom/shift-ai");
    expect(getShiftBinary()).toBe("/custom/shift-ai");
  });
});
