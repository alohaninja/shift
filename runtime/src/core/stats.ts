/**
 * Stats recording — writes RunRecord entries to ~/.shift/stats.jsonl
 * in the same format used by the shift-ai CLI, so `shift-ai gain`
 * sees proxy/middleware stats alongside CLI stats.
 *
 * The proxy previously passed --no-stats to shift-ai to avoid per-image
 * noise. This module records one entry per REQUEST instead, matching
 * the granularity the CLI uses (one entry per invocation).
 */

import { appendFile, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

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
 * The optional `source` field is a backwards-compatible extension
 * that lets `shift-ai gain` distinguish CLI vs proxy vs middleware
 * records if it ever wants to. The Rust deserializer uses
 * `#[serde(default)]` semantics so unknown fields are ignored.
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

/** Default stats file path: ~/.shift/stats.jsonl */
export function defaultStatsPath(): string {
  return join(homedir(), ".shift", "stats.jsonl");
}

/**
 * Append a RunRecord to the stats file and accumulate into session stats.
 *
 * Writes are fire-and-forget in the proxy — a failed write should never
 * block or fail a request. Errors are logged to stderr and swallowed.
 */
export async function recordRun(
  record: RunRecord,
  statsPath?: string,
): Promise<void> {
  // Always accumulate in-memory session stats
  accumulateSession(record);

  const filePath = statsPath ?? defaultStatsPath();
  try {
    // Ensure ~/.shift directory exists
    const dir = join(filePath, "..");
    await mkdir(dir, { recursive: true });

    const line = JSON.stringify(record) + "\n";
    await appendFile(filePath, line, { mode: 0o600 });
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
 * 1 image was modified. For token estimation we leave zeros — the CLI
 * records accurate per-image token counts via its pipeline report;
 * proxy records capture byte savings which is the primary metric.
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
    duration_ms: Math.round(input.durationMs),
    action_counts: wasSaved ? [["optimize", 1]] : [],
    source: input.source,
  };
}
