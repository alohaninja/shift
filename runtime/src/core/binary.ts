/**
 * Lazy detection of the shift-ai binary.
 *
 * Checks once on first use and caches the result.
 * If shift-ai is not installed, logs a warning and returns false.
 */

import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

/** Cache keyed by resolved binary path. */
const _cache = new Map<string, { available: boolean; promise?: Promise<boolean> }>();
let _warned = false;
let _lastPath = "shift-ai";

/**
 * Check whether the shift-ai binary is available.
 * Result is cached per binary path. Concurrent callers for the same
 * path share a single in-flight check.
 */
export async function isShiftAvailable(
  binaryPath?: string,
): Promise<boolean> {
  const bin = binaryPath ?? "shift-ai";
  const cached = _cache.get(bin);

  // Return cached result if available and not in-flight
  if (cached && !cached.promise) return cached.available;

  // Return in-flight promise if another caller is already checking
  if (cached?.promise) return cached.promise;

  const entry: { available: boolean; promise?: Promise<boolean> } = { available: false };
  entry.promise = (async () => {
    try {
      await execFileAsync(bin, ["--version"], { timeout: 5_000 });
      entry.available = true;
      _lastPath = bin;
    } catch {
      entry.available = false;
      if (!_warned) {
        _warned = true;
        console.warn(
          "[@shift-preflight/runtime] shift-ai binary not found. Images will pass through unoptimized.\n" +
            "  Install: brew install alohaninja/shift/shift-ai\n" +
            "  More info: https://shift-ai.dev",
        );
      }
    }
    // Clear the in-flight promise so future calls return the cached result
    delete entry.promise;
    return entry.available;
  })();

  _cache.set(bin, entry);
  return entry.promise;
}

/** Get the resolved binary path (defaults to "shift-ai"). */
export function getShiftBinary(): string {
  return _lastPath;
}

/**
 * Reset cached state. Useful for testing.
 * @internal
 */
export function _resetBinaryCache(): void {
  _cache.clear();
  _warned = false;
  _lastPath = "shift-ai";
}
