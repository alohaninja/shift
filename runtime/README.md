# @shift-preflight/runtime

[![npm](https://img.shields.io/npm/v/@shift-preflight/runtime)](https://www.npmjs.com/package/@shift-preflight/runtime)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

Multimodal preflight for any AI agent — transparent image optimization before images reach the LLM.

Two integration modes:

1. **AI SDK Middleware** — in-process, zero-config for any [Vercel AI SDK](https://sdk.vercel.ai) app (OpenCode, Next.js, custom agents)
2. **HTTP Proxy** — transparent reverse proxy for any agent in any language (Claude Code, Codex CLI, Gemini CLI, Python, curl)

Both use [SHIFT](https://shift-ai.dev/) (`shift-ai` CLI) as the optimization engine.

## Install

```bash
npm install @shift-preflight/runtime

# SHIFT CLI is required for optimization (graceful no-op if missing)
brew install alohaninja/shift/shift-ai
```

## Mode 1: AI SDK Middleware

```typescript
import { shiftMiddleware } from "@shift-preflight/runtime";
import { wrapLanguageModel, generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";

const model = wrapLanguageModel({
  model: anthropic("claude-sonnet-4-20250514"),
  middleware: shiftMiddleware({ mode: "balanced" }),
});

const result = await generateText({
  model,
  messages: [{
    role: "user",
    content: [
      { type: "text", text: "What's in this screenshot?" },
      { type: "file", data: largeBase64, mediaType: "image/png" },
    ],
  }],
});
```

The middleware intercepts `transformParams` and optimizes all images in all messages (user, assistant, tool results) before they reach the provider. Already-optimized images are tagged and skipped on subsequent turns.

### Configuration

```typescript
shiftMiddleware({
  mode: "balanced",     // "performance" | "balanced" | "economy"
  minSize: 100_000,     // skip images < 100KB (default)
  disabled: false,      // kill switch
  provider: undefined,  // auto-detected from model
  model: undefined,     // override model for SHIFT profile
  binary: undefined,    // path to shift-ai binary
  onOptimize: (metrics) => {
    console.log(`Saved ${metrics[0].savedBytes} bytes`);
  },
});
```

## Mode 2: HTTP Proxy

```bash
npx @shift-preflight/runtime proxy --port 8787 --mode balanced
```

Then point your agent at `http://localhost:8787`:

```bash
# Claude Code
export ANTHROPIC_BASE_URL=http://localhost:8787

# Codex CLI
export OPENAI_BASE_URL=http://localhost:8787

# Gemini CLI
export GEMINI_API_BASE=http://localhost:8787

# Python
client = Anthropic(base_url="http://localhost:8787")
client = OpenAI(base_url="http://localhost:8787/v1")
```

The proxy intercepts requests, optimizes images via SHIFT, and forwards to the real API. Auth headers pass through. SSE streams pipe directly.

### Routes

| Route | Provider |
|---|---|
| `POST /v1/messages` | Anthropic |
| `POST /v1/chat/completions` | OpenAI |
| `POST /v1beta/models/*` | Google (passthrough, native support pending) |

### Programmatic

```typescript
import { startProxy } from "@shift-preflight/runtime/proxy";

const server = await startProxy({
  port: 8787,
  mode: "balanced",
  verbose: true,
});
```

## Drive Modes

| Mode | Behavior |
|---|---|
| **performance** | Only enforce hard provider limits. Preserve fidelity. |
| **balanced** | Resize oversized, recompress bloated. Remove obvious waste. |
| **economy** | Downscale everything to 1024px. Minimize tokens. |

## Provider Support

| Provider | Middleware | Proxy | Constraints |
|---|:---:|:---:|---|
| **Anthropic** | Yes | Yes | 5MB max, 1.15 MP, 8000px |
| **OpenAI** | Yes | Yes | 20MB max, 2048px, tile tokens |
| **Google** | Yes | Passthrough* | 20MB max, 3072px |

\* Google proxy optimization pending native SHIFT support for Gemini payload format.

## Requirements

- Node.js >= 18
- `shift-ai` CLI ([install](https://shift-ai.dev)) — graceful no-op if missing

## License

Apache-2.0
