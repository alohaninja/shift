#!/usr/bin/env node
/**
 * CLI entry point for @shift-ai/runtime.
 *
 * Usage:
 *   npx @shift-ai/runtime proxy [options]
 *   shift-runtime proxy --port 8787 --mode balanced --verbose
 */

import type { DriveMode } from "./core/types.js";

async function main() {
  const args = process.argv.slice(2);
  const command = args[0];

  if (command === "proxy") {
    const port = getFlag(args, "--port", "8787");
    const mode = getFlag(args, "--mode", "balanced") as DriveMode;
    const verbose = args.includes("--verbose") || args.includes("-v");

    const { startProxy } = await import("./proxy/index.js");
    await startProxy({ port: parseInt(port, 10), mode, verbose });
  } else if (command === "help" || command === "--help" || command === "-h" || !command) {
    printHelp();
  } else {
    console.error(`Unknown command: ${command}\n`);
    printHelp();
    process.exit(1);
  }
}

function getFlag(args: string[], flag: string, defaultValue: string): string {
  const idx = args.indexOf(flag);
  if (idx === -1 || idx + 1 >= args.length) return defaultValue;
  return args[idx + 1] ?? defaultValue;
}

function printHelp() {
  console.log(`
@shift-ai/runtime — Multimodal preflight for any AI agent

USAGE:
  shift-runtime proxy [OPTIONS]

COMMANDS:
  proxy       Start the SHIFT proxy server

OPTIONS:
  --port <PORT>       Port to listen on (default: 8787)
  --mode <MODE>       Drive mode: performance, balanced, economy (default: balanced)
  --verbose, -v       Enable verbose logging
  --help, -h          Show this help message

EXAMPLES:
  # Start proxy with defaults
  npx @shift-ai/runtime proxy

  # Start proxy on port 9000 in economy mode
  npx @shift-ai/runtime proxy --port 9000 --mode economy

  # Then point your agent at the proxy:
  export ANTHROPIC_BASE_URL=http://localhost:8787
  export OPENAI_BASE_URL=http://localhost:8787

MORE INFO:
  https://shift-ai.dev
  https://github.com/alohaninja/shift
`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
