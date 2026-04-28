/**
 * Stats recording — writes RunRecord entries to ~/.shift/stats.jsonl
 * in the same format used by the shift-ai CLI, so `shift-ai gain`
 * sees proxy/middleware stats alongside CLI stats.
 *
 * The proxy previously passed --no-stats to shift-ai to avoid per-image
 * noise. This module records one entry per REQUEST instead, matching
 * the granularity the CLI uses (one entry per invocation).
 *
 * NOTE: File rotation/retention purge is handled by the Rust CLI
 * (shift-core/src/stats.rs purge_old_records — auto-purges >90 day
 * records when the file exceeds 50KB). The TS writer is append-only
 * and relies on the CLI's purge cycle. This is intentional — the CLI
 * is the canonical stats manager; the TS side is a lightweight recorder.
 * TODO: Port retention purge to TS if the proxy is ever used standalone
 * without the CLI installed.
 */

// Node-specific modules are loaded lazily via loadNodeDeps() so that
// importing this module in non-Node environments (e.g., bundled plugins,
// edge runtimes, Bun without node compat) doesn't crash at load time.
// The types, buildRunRecord(), and session accumulator are all pure JS
// and work everywhere — only recordRun()'s file write needs Node APIs.

interface NodeDeps {
  appendFile: typeof import("node:fs/promises").appendFile;
  lstat: typeof import("node:fs/promises").lstat;
  mkdir: typeof import("node:fs/promises").mkdir;
  homedir: typeof import("node:os").homedir;
  dirname: typeof import("node:path").dirname;
  join: typeof import("node:path").join;
}

let _nodeDeps: NodeDeps | null | undefined; // undefined = not yet loaded
let _loadPromise: Promise<NodeDeps | null> | undefined;

/** Path segments for the default stats file (extracted to avoid duplication). */
const STATS_DIR = ".shift";
const STATS_FILE = "stats.jsonl";

async function loadNodeDeps(): Promise<NodeDeps | null> {
  if (_nodeDeps !== undefined) return _nodeDeps;
  // Cache the in-flight promise so concurrent callers share one load.
  if (!_loadPromise) {
    _loadPromise = (async (): Promise<NodeDeps | null> => {
      try {
        const [fs, os, path] = await Promise.all([
          import("node:fs/promises"),
          import("node:os"),
          import("node:path"),
        ]);
        _nodeDeps = {
          appendFile: fs.appendFile,
          lstat: fs.lstat,
          mkdir: fs.mkdir,
          homedir: os.homedir,
          dirname: path.dirname,
          join: path.join,
        };
      } catch {
        // Not in a Node-compatible environment — file writes will no-op
        console.warn(
          "[shift-runtime] Node APIs unavailable — stats file writes disabled",
        );
        _nodeDeps = null;
      }
      return _nodeDeps;
    })();
  }
  return _loadPromise;
}

/** @internal — reset for testing */
export function _resetNodeDeps(): void {
  _nodeDeps = undefined;
  _loadPromise = undefined;
}

// ────────────────────────────────────────────────────────────────────
// Types — mirrors the Rust RunRecord in shift-core/src/stats.rs
// ────────────────────────────────────────────────────────────────────

export interface TokenSavings {
  openai_before: number;
  openai_after: number;
  anthropic_before: number;
  anthropic_after: number;
}

/**
 * A single stats record, compatible with the Rust `RunRecord` struct.
 *
 * The optional `source` field is a backwards-compatible extension that
 * lets `shift-ai gain` distinguish CLI vs proxy vs middleware records
 * if it ever wants to. Serde's default `Deserialize` impl silently
 * ignores unknown fields (no `#[serde(deny_unknown_fields)]` on
 * RunRecord), so the Rust reader will not reject records with `source`.
 *
 * CAVEAT: The `source` field will be dropped when the Rust CLI's
 * `purge_old_records()` re-serializes records through the Rust struct,
 * since the Rust struct has no `source` field. This is acceptable —
 * the field is informational, not load-bearing. If it needs to survive
 * purges, add `source: Option<String>` to the Rust RunRecord.
 *
 * TODO: Add a cross-language roundtrip test in CI that writes a
 * RunRecord from TS and reads it back via the Rust `load_records()`.
 */
export interface RunRecord {
  /** ISO 8601 timestamp. */
  timestamp: string;
  /** Date portion (YYYY-MM-DD) for daily aggregation. */
  date: string;
  /** Provider used. */
  provider: string;
  /** Number of images processed. */
  images: number;
  /** Number of images modified. */
  modified: number;
  /** Number of images dropped. */
  dropped: number;
  /** Number of SVGs rasterized. */
  svgs_rasterized: number;
  /** Total bytes before optimization. */
  bytes_before: number;
  /** Total bytes after optimization. */
  bytes_after: number;
  /** Token savings. */
  token_savings: TokenSavings;
  /** Pipeline execution time in milliseconds. */
  duration_ms: number;
  /** Per-action counts: [action_name, count][]. */
  action_counts: Array<[string, number]>;
  /** Source of this record (backwards-compatible extension). */
  source?: "cli" | "proxy" | "middleware";
}

// ────────────────────────────────────────────────────────────────────
// In-memory session accumulator for the /stats endpoint
// ────────────────────────────────────────────────────────────────────

export interface SessionStats {
  startedAt: string;
  totalRequests: number;
  totalImages: number;
  totalImagesModified: number;
  totalBytesSaved: number;
  tokenSavings: TokenSavings;
}

let _session: SessionStats = createFreshSession();

function createFreshSession(): SessionStats {
  return {
    startedAt: new Date().toISOString(),
    totalRequests: 0,
    totalImages: 0,
    totalImagesModified: 0,
    totalBytesSaved: 0,
    tokenSavings: {
      openai_before: 0,
      openai_after: 0,
      anthropic_before: 0,
      anthropic_after: 0,
    },
  };
}

/** Get current session stats (for the /stats endpoint). */
export function getSessionStats(): SessionStats {
  return { ..._session, tokenSavings: { ..._session.tokenSavings } };
}

/** Reset session stats (for testing). */
export function resetSessionStats(): void {
  _session = createFreshSession();
}

function accumulateSession(record: RunRecord): void {
  _session.totalRequests++;
  _session.totalImages += record.images;
  _session.totalImagesModified += record.modified;
  _session.totalBytesSaved += record.bytes_before - record.bytes_after;
  _session.tokenSavings.openai_before += record.token_savings.openai_before;
  _session.tokenSavings.openai_after += record.token_savings.openai_after;
  _session.tokenSavings.anthropic_before +=
    record.token_savings.anthropic_before;
  _session.tokenSavings.anthropic_after +=
    record.token_savings.anthropic_after;
}

// ────────────────────────────────────────────────────────────────────
// File writer
// ────────────────────────────────────────────────────────────────────

/**
 * Default stats file path: ~/.shift/stats.jsonl
 *
 * Returns the cached path if Node deps have already been loaded,
 * otherwise triggers an async lazy load. Returns empty string in
 * non-Node environments. The main code path in recordRun() constructs
 * the path internally via lazy deps — this function exists primarily
 * for external callers and tests.
 */
export async function defaultStatsPath(): Promise<string> {
  const deps = await loadNodeDeps();
  if (!deps) return "";
  return deps.join(deps.homedir(), STATS_DIR, STATS_FILE);
}

/**
 * Append a RunRecord to the stats file and accumulate into session stats.
 *
 * Writes are fire-and-forget in the proxy — a failed write should never
 * block or fail a request. Errors are logged to stderr and swallowed.
 *
 * Security: Rejects symlinks on the stats file to match the Rust CLI's
 * O_NOFOLLOW behavior (shift-core/src/stats.rs:168-182). Uses lstat to
 * detect symlinks before opening. This is a TOCTOU check (not as strong
 * as O_NOFOLLOW on the open call itself), but matches the Rust non-Unix
 * fallback path and prevents the most common symlink attacks.
 *
 * Permissions: The `mode: 0o600` flag only applies when creating a new
 * file. If the file already exists with different permissions, they are
 * NOT corrected. This matches the Rust CLI's behavior.
 *
 * @param statsPath - Override path for testing. @internal
 */
export async function recordRun(
  record: RunRecord,
  statsPath?: string,
): Promise<void> {
  // Always accumulate in-memory session stats (works in any environment)
  accumulateSession(record);

  // Load Node APIs lazily — no-op in non-Node environments
  const deps = await loadNodeDeps();
  if (!deps) return; // silently skip file write in non-Node environments

  const filePath = statsPath ?? deps.join(deps.homedir(), STATS_DIR, STATS_FILE);
  try {
    // Ensure parent directory exists
    const dir = deps.dirname(filePath);
    await deps.mkdir(dir, { recursive: true });

    // Reject symlinks (defense-in-depth, matching Rust's O_NOFOLLOW)
    try {
      const stat = await deps.lstat(filePath);
      if (stat.isSymbolicLink()) {
        console.warn(
          `[shift-runtime] Refusing to write stats: ${filePath} is a symlink`,
        );
        return;
      }
    } catch {
      // File doesn't exist yet — that's fine, appendFile will create it
    }

    const line = JSON.stringify(record) + "\n";
    await deps.appendFile(filePath, line, { mode: 0o600 });
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    console.warn(`[shift-runtime] Failed to write stats: ${msg}`);
  }
}

// ────────────────────────────────────────────────────────────────────
// Record builder — creates a RunRecord from proxy request data
// ────────────────────────────────────────────────────────────────────

export interface RecordRunInput {
  provider: string;
  originalBytes: number;
  optimizedBytes: number;
  durationMs: number;
  source?: "proxy" | "middleware";
}

/**
 * Build a RunRecord from proxy/middleware request-level data.
 *
 * The proxy doesn't have per-image breakdowns (shift-ai is called on
 * the whole payload), so we estimate: if bytes were saved, at least
 * 1 image was modified. The actual image count may be higher for
 * multi-image payloads — this is a known limitation that causes
 * `shift-ai gain` to undercount images for proxy traffic. The byte
 * savings metrics (bytes_before, bytes_after) are always accurate.
 *
 * For token estimation we leave zeros — the CLI records accurate
 * per-image token counts via its pipeline report; proxy records
 * capture byte savings which is the primary metric.
 */
export function buildRunRecord(input: RecordRunInput): RunRecord {
  const now = new Date();
  const wasSaved = input.optimizedBytes < input.originalBytes;
  return {
    timestamp: now.toISOString(),
    date: now.toISOString().slice(0, 10),
    provider: input.provider,
    images: wasSaved ? 1 : 0, // conservative: at least 1 if optimized
    modified: wasSaved ? 1 : 0,
    dropped: 0,
    svgs_rasterized: 0,
    bytes_before: input.originalBytes,
    bytes_after: input.optimizedBytes,
    token_savings: {
      openai_before: 0,
      openai_after: 0,
      anthropic_before: 0,
      anthropic_after: 0,
    },
    // Clamp to 0 to avoid negative values from NTP clock adjustments.
    // Rust deserializes duration_ms as u64 which rejects negatives.
    duration_ms: Math.max(0, Math.round(input.durationMs)),
    action_counts: wasSaved ? [["optimize", 1]] : [],
    source: input.source,
  };
}
