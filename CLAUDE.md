# SHIFT — AI Assistant Guide

SHIFT is a multimodal preflight layer that optimizes images in AI API request payloads before they are sent. Single Rust binary, pipe-composable, works with OpenAI and Anthropic.

## shift-ai CLI

The binary is `shift-ai`, installed via Homebrew (`brew install alohaninja/shift/shift-ai`), crates.io (`cargo install shift-preflight-cli`), or local build (`cargo install --path shift-cli`).

Key commands:

- `shift-ai preflight <file>` — inspect payload, preview optimizations, output structured JSON report. **Read-only** — does not modify the payload.
- `shift-ai <file>` — transform payload, write optimized JSON to stdout.
- `shift-ai gain` — view cumulative token savings across all recorded runs.

When asked to optimize images for AI model API requests, load the `shift-ai-preflight` skill at `.agents/skills/shift-ai-preflight/SKILL.md`.

## Project structure

- `shift-core/` — library crate (`shift-preflight`): pipeline, image inspection, policy, cost estimation
- `shift-cli/` — binary crate (`shift-preflight-cli`): CLI entry point, subcommands
- `runtime/` — TypeScript package (`@shift-preflight/runtime`): AI SDK middleware + HTTP proxy
- `profiles/` — provider constraint profiles (JSON)
- `assets/` — README images

## Build and test

### Rust (CLI + core)

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo test                     # run all tests
cargo check -p shift-preflight-cli  # quick type check on CLI only
```

### Runtime (TypeScript)

```bash
cd runtime
npm install                    # install dependencies
npm run build                  # build ESM + DTS
npm run typecheck              # TypeScript type check
npm test                       # run tests (vitest)
```

The runtime has two integration modes:
- **AI SDK Middleware** — `shiftMiddleware()` for any Vercel AI SDK app (OpenCode, Next.js)
- **HTTP Proxy** — `npx @shift-preflight/runtime proxy` for any agent (Claude Code, Codex CLI, Gemini CLI)

Both call the `shift-ai` CLI under the hood. Graceful no-op when `shift-ai` is not installed.

## Naming

The workspace has two crates and one npm package with names that differ from directory names:

| Directory | Package name | Binary / Entry |
|-----------|-----------|--------|
| `shift-core/` | `shift-preflight` | (library) |
| `shift-cli/` | `shift-preflight-cli` | `shift-ai` |
| `runtime/` | `@shift-preflight/runtime` | `shift-runtime` (CLI) |
