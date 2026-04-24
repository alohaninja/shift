/**
 * Lazy detection of the shift-ai binary.
 *
 * Checks once on first use and caches the result.
 * If shift-ai is not installed, logs a warning and returns false.
 */

import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

let _checked = false;
let _available = false;
let _path = "shift-ai";
let _warned = false;

/**
 * Check whether the shift-ai binary is available.
 * Result is cached after first call.
 */
export async function isShiftAvailable(
  binaryPath?: string,
): Promise<boolean> {
  if (_checked && binaryPath === undefined) return _available;

  const bin = binaryPath ?? "shift-ai";
  try {
    await execFileAsync(bin, ["--version"], { timeout: 5_000 });
    _available = true;
    _path = bin;
  } catch {
    _available = false;
    if (!_warned) {
      _warned = true;
      console.warn(
        "[@shift-ai/runtime] shift-ai binary not found. Images will pass through unoptimized.\n" +
          "  Install: brew install alohaninja/shift/shift-ai\n" +
          "  More info: https://shift-ai.dev",
      );
    }
  }
  _checked = true;
  return _available;
}

/** Get the resolved binary path (defaults to "shift-ai"). */
export function getShiftBinary(): string {
  return _path;
}

/**
 * Reset cached state. Useful for testing.
 * @internal
 */
export function _resetBinaryCache(): void {
  _checked = false;
  _available = false;
  _path = "shift-ai";
  _warned = false;
}
