/**
 * Maps AI SDK model identifiers and proxy routes to SHIFT provider names.
 */

import type { ShiftProvider } from "./types.js";

/** Known AI SDK provider IDs → SHIFT provider. */
const PROVIDER_ID_MAP: Record<string, ShiftProvider> = {
  anthropic: "anthropic",
  "anthropic.messages": "anthropic",
  openai: "openai",
  "openai.chat": "openai",
  google: "google",
  "google-generative-ai": "google",
  "google-vertex": "google",
  "amazon-bedrock": "anthropic", // Bedrock Claude uses Anthropic constraints
};

/** Model ID prefix patterns → SHIFT provider. */
const MODEL_PREFIX_MAP: [string, ShiftProvider][] = [
  ["claude-", "anthropic"],
  ["gpt-", "openai"],
  ["o1", "openai"],
  ["o3", "openai"],
  ["o4", "openai"],
  ["gemini-", "google"],
];

/**
 * Detect the SHIFT provider from an AI SDK model object.
 *
 * Checks `model.provider` first (the AI SDK provider ID string),
 * then falls back to model ID prefix matching.
 */
export function detectProviderFromModel(model: {
  provider?: string;
  modelId?: string;
}): ShiftProvider | undefined {
  // Try provider ID
  if (model.provider) {
    // AI SDK provider strings can be like "anthropic.chat" — try full, then base
    const direct = PROVIDER_ID_MAP[model.provider];
    if (direct) return direct;

    const base = model.provider.split(".")[0];
    if (base) {
      const mapped = PROVIDER_ID_MAP[base];
      if (mapped) return mapped;
    }
  }

  // Try model ID prefix
  if (model.modelId) {
    const id = model.modelId.toLowerCase();
    for (const [prefix, provider] of MODEL_PREFIX_MAP) {
      if (id.startsWith(prefix)) return provider;
    }
  }

  return undefined;
}

/** Proxy route path → SHIFT provider. */
const ROUTE_MAP: [RegExp, ShiftProvider][] = [
  [/^\/v1\/messages/, "anthropic"],
  [/^\/v1\/chat\/completions/, "openai"],
  [/^\/v1beta\/models\//, "google"],
  [/^\/v1\/(beta\/)?models\//, "google"],
];

/**
 * Detect the SHIFT provider from an HTTP request path (proxy mode).
 */
export function detectProviderFromRoute(path: string): ShiftProvider | undefined {
  for (const [pattern, provider] of ROUTE_MAP) {
    if (pattern.test(path)) return provider;
  }
  return undefined;
}
