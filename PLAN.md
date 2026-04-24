# SHIFT Build Plan

## Smart Hybrid Input Filtering & Transformation

SHIFT is a multimodal preflight layer that automatically adapts inputs (images, video, audio, documents, text) before they are sent to an AI model. It ensures requests never fail due to invalid payloads, inputs are optimized for cost/latency/quality, and behavior is tunable via "drive modes."

---

## Decisions

| Decision | Choice |
|---|---|
| Language | Rust |
| Scope | Full modality scaffold, image-only implementation |
| Distribution | CLI-first + library, skills.sh compatible |
| Providers | OpenAI + Anthropic |
| Image processing | `image` crate (pure Rust, zero external deps) |
| Input handling | Base64 + URL auto-detection |
| CLI model | Pipe + file mode (stdin/stdout composable) |

---

## Repository Structure

```
shift/
├── Cargo.toml                    # Workspace root
├── Cargo.lock
├── README.md
├── PLAN.md
├── LICENSE
├── .gitignore
├── .github/
│   └── workflows/
│       └── ci.yml                # cargo check + clippy + test
├── shift-core/                   # Library crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                # Public API surface
│       ├── mode.rs               # DriveMode enum (Performance, Balanced, Economy)
│       ├── inspector/
│       │   ├── mod.rs            # Inspector trait + dispatch
│       │   ├── image.rs          # Image metadata extraction
│       │   ├── video.rs          # Stub (v2)
│       │   ├── audio.rs          # Stub (v2)
│       │   └── document.rs       # Stub (v2)
│       ├── policy/
│       │   ├── mod.rs            # Policy engine
│       │   ├── provider.rs       # Provider profiles (OpenAI, Anthropic)
│       │   └── rules.rs          # Mode-based rule resolution
│       ├── transformer/
│       │   ├── mod.rs            # Transformer trait + dispatch
│       │   ├── image.rs          # Image transforms (resize, recompress)
│       │   ├── video.rs          # Stub (v2)
│       │   ├── audio.rs          # Stub (v2)
│       │   └── document.rs       # Stub (v2)
│       ├── payload/
│       │   ├── mod.rs            # Payload parsing + reconstruction
│       │   ├── openai.rs         # OpenAI message format parser
│       │   └── anthropic.rs      # Anthropic message format parser
│       ├── pipeline.rs           # Core: inspect -> policy -> transform
│       └── report.rs             # Transformation report
├── shift-cli/                    # Binary crate
│   ├── Cargo.toml
│   └── src/
│       └── main.rs               # CLI entry point (clap)
├── profiles/                     # Provider profile JSON files
│   ├── openai.json
│   └── anthropic.json
└── tests/
    ├── fixtures/                 # Test images + sample payloads
    │   ├── oversized.png
    │   ├── small.jpg
    │   ├── openai_request.json
    │   └── anthropic_request.json
    └── integration/
        └── pipeline_test.rs
```

---

## Module Design

### shift-core (library crate)

The library is the heart. Everything is usable without the CLI.

#### `mode.rs` — Drive Modes

```rust
pub enum DriveMode {
    Performance,  // minimal transforms, only hard limits
    Balanced,     // moderate optimization (default)
    Economy,      // aggressive optimization
}
```

#### `inspector/` — Payload Inspection

- **Trait:** `Inspector` with `fn inspect(&self, input: &[u8]) -> Result<Metadata>`
- **`image.rs`:** Uses `image` crate to extract dimensions, format, file size, detect encoding (base64 vs raw bytes). Handles URL-referenced images by fetching them.
- **Stubs:** `video.rs`, `audio.rs`, `document.rs` return `Err("not yet supported")`

#### `policy/` — Constraint Evaluation

- **`provider.rs`:** Loads provider profiles from embedded JSON or external files:
  ```rust
  pub struct ProviderProfile {
      pub name: String,
      pub max_images: usize,
      pub max_image_dim: u32,
      pub max_image_size_bytes: usize,
      pub supported_formats: Vec<String>,
      pub max_tokens_per_image: Option<u32>,
  }
  ```

- **`rules.rs`:** Given `Metadata` + `ProviderProfile` + `DriveMode`, produces a list of `Action`s:
  ```rust
  pub enum Action {
      Resize { target_dim: u32 },
      Recompress { quality: u8 },
      ConvertFormat { to: ImageFormat },
      Drop { reason: String },
      CaptionFallback,  // v2
      Pass,             // no change needed
  }
  ```

#### `transformer/` — Apply Transformations

- **Trait:** `Transformer` with `fn transform(&self, input: &[u8], action: &Action) -> Result<Vec<u8>>`
- **`image.rs`:** Implements resize (preserve aspect ratio), JPEG recompression, format conversion
- **Stubs:** other modalities

#### `payload/` — Message Format Parsing

- **`openai.rs`:** Parse/reconstruct OpenAI chat completion messages with `image_url` content parts (both base64 `data:` URIs and `https://` URLs)
- **`anthropic.rs`:** Parse/reconstruct Anthropic messages with `image` content blocks (`base64` and `url` source types)

#### `pipeline.rs` — Core Pipeline

```
inspect -> policy evaluate -> transform -> reconstruct payload
```

Single entry point:
```rust
pub fn process(
    payload: &Value,
    provider: &str,
    mode: DriveMode,
) -> Result<(Value, Report)>
```

Returns transformed payload + a report of what changed.

#### `report.rs` — Transformation Report

```rust
pub struct Report {
    pub original_size: usize,
    pub transformed_size: usize,
    pub actions_taken: Vec<ActionRecord>,
    pub images_processed: usize,
    pub images_dropped: usize,
    pub warnings: Vec<String>,
}
```

---

### shift-cli (binary crate)

#### CLI Interface

```
shift [OPTIONS] [FILE]

Arguments:
  [FILE]  Input file (JSON request payload). Reads stdin if omitted.

Options:
  -p, --provider <PROVIDER>  Target provider [default: openai]
                              [possible values: openai, anthropic]
  -m, --mode <MODE>          Drive mode [default: balanced]
                              [possible values: performance, balanced, economy]
  -o, --output <FORMAT>      Output format [default: json]
                              [possible values: json, report]
  --dry-run                  Show what would change without modifying
  --profile <FILE>           Custom provider profile JSON
  -v, --verbose              Verbose output
  -h, --help                 Print help
  -V, --version              Print version
```

#### Usage Examples

```bash
# Pipe mode — transform an OpenAI request
cat request.json | shift -p openai -m balanced > safe_request.json

# File mode
shift request.json -p anthropic -m economy > safe_request.json

# Dry run — see what would change
shift request.json --dry-run -o report

# Use with curl
shift request.json -p openai | curl -X POST https://api.openai.com/v1/chat/completions \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d @-
```

---

## Provider Profiles (v1)

### `profiles/openai.json`

```json
{
  "name": "openai",
  "models": {
    "gpt-4o": {
      "max_images": 10,
      "max_image_dim": 2048,
      "max_image_size_bytes": 20971520,
      "supported_formats": ["png", "jpeg", "gif", "webp"]
    },
    "gpt-4o-mini": {
      "max_images": 10,
      "max_image_dim": 2048,
      "max_image_size_bytes": 20971520,
      "supported_formats": ["png", "jpeg", "gif", "webp"]
    },
    "gpt-4.1": {
      "max_images": 10,
      "max_image_dim": 2048,
      "max_image_size_bytes": 20971520,
      "supported_formats": ["png", "jpeg", "gif", "webp"]
    }
  },
  "default": {
    "max_images": 10,
    "max_image_dim": 2048,
    "max_image_size_bytes": 20971520,
    "supported_formats": ["png", "jpeg", "gif", "webp"]
  }
}
```

### `profiles/anthropic.json`

```json
{
  "name": "anthropic",
  "models": {
    "claude-sonnet-4-20250514": {
      "max_images": 20,
      "max_image_dim": 8000,
      "max_image_size_bytes": 5242880,
      "max_image_megapixels": 1.15,
      "supported_formats": ["png", "jpeg", "gif", "webp"]
    }
  },
  "default": {
    "max_images": 20,
    "max_image_dim": 8000,
    "max_image_size_bytes": 5242880,
    "max_image_megapixels": 1.15,
    "supported_formats": ["png", "jpeg", "gif", "webp"]
  }
}
```

---

## Dependencies

| Crate | Purpose |
|---|---|
| `clap` 4 (derive) | CLI argument parsing |
| `serde` + `serde_json` | JSON parsing/serialization |
| `image` 0.25 | Image decoding, resizing, encoding |
| `base64` 0.22 | Base64 encode/decode |
| `anyhow` 1 | Error handling |
| `minreq` 2 | HTTP client (fetch URL-referenced images) |
| `url` 2 | URL parsing |

---

## Build Phases

### Phase 1: Skeleton

1. `Cargo.toml` (workspace)
2. `shift-core/Cargo.toml`
3. `shift-cli/Cargo.toml`
4. `.gitignore`
5. `shift-core/src/lib.rs` (re-exports)
6. `shift-core/src/mode.rs` (DriveMode enum)
7. `shift-cli/src/main.rs` (clap skeleton, reads stdin/file, prints)
8. Verify it compiles

### Phase 2: Inspector

9. `shift-core/src/inspector/mod.rs` (trait + dispatch)
10. `shift-core/src/inspector/image.rs` (extract metadata from bytes, base64, URLs)
11. Stubs for video/audio/document
12. Unit tests for image inspector

### Phase 3: Policy Engine

13. `profiles/openai.json` + `profiles/anthropic.json`
14. `shift-core/src/policy/provider.rs` (load + parse profiles)
15. `shift-core/src/policy/rules.rs` (mode-based rule evaluation)
16. `shift-core/src/policy/mod.rs` (policy engine)
17. Unit tests for policy

### Phase 4: Transformer

18. `shift-core/src/transformer/mod.rs` (trait)
19. `shift-core/src/transformer/image.rs` (resize, recompress)
20. Stubs for other modalities
21. Unit tests for image transformer

### Phase 5: Payload Parsing

22. `shift-core/src/payload/mod.rs`
23. `shift-core/src/payload/openai.rs` (parse/reconstruct OpenAI messages)
24. `shift-core/src/payload/anthropic.rs` (parse/reconstruct Anthropic messages)
25. Unit tests for payload parsing

### Phase 6: Pipeline + Report

26. `shift-core/src/report.rs`
27. `shift-core/src/pipeline.rs` (wire it all together)
28. Integration tests with fixture payloads

### Phase 7: CLI Polish

29. Complete `shift-cli/src/main.rs` (wire pipeline, output formatting)
30. End-to-end test: real JSON payload through CLI

### Phase 8: Packaging

31. `README.md`
32. `.github/workflows/ci.yml`
33. Initial commit + push

### Phase 9: Runtime — TypeScript Package (`@shift-preflight/runtime`)

34. `runtime/package.json` + tsconfig + tsup + vitest scaffolding
35. `runtime/src/core/` — binary detection, data-convert, provider-detect, metrics, optimizer
36. `runtime/src/middleware/` — AI SDK `LanguageModelV3Middleware` with `transformParams`
37. `runtime/src/proxy/` — Hono reverse proxy with Anthropic, OpenAI, Google routes
38. `runtime/src/cli.ts` — `npx @shift-preflight/runtime proxy` entry point
39. `profiles/google.json` — Gemini provider constraints
40. Tests for core, middleware, proxy (51 tests)
41. `runtime/README.md` with middleware + proxy usage docs

---

## Key Design Decisions

1. **Workspace crate structure** (`shift-core` + `shift-cli`) — library is usable independently from the CLI
2. **Provider profiles as embedded JSON** — compiled into the binary via `include_str!()`, with `--profile` override for custom ones
3. **Pure Rust image processing** — no external dependencies, single static binary
4. **Pipe-friendly CLI** — reads stdin, writes stdout, composable with `curl`/`jq`/other tools
5. **Report as sidecar** — transformation report goes to stderr (or `--output report` mode), payload to stdout
6. **Anthropic megapixel constraint** — Anthropic limits by megapixels (1.15MP), not just dimensions. The policy engine handles this.

---

## Success Criteria

- [ ] `cargo build` produces a single `shift` binary
- [ ] Oversized image in OpenAI payload is auto-resized
- [ ] Anthropic megapixel constraint is enforced
- [ ] Three drive modes produce different transformation behavior
- [ ] Pipe mode works: `cat request.json | shift > safe.json`
- [ ] Dry-run mode shows report without modifying payload
- [ ] All unit tests pass
- [ ] CI runs cargo check + clippy + test
- [ ] `@shift-preflight/runtime` builds and typechecks cleanly
- [ ] AI SDK middleware transparently optimizes images in `transformParams`
- [ ] HTTP proxy forwards Anthropic/OpenAI requests with optimized images
- [ ] Proxy passes through auth headers and SSE streams unchanged
- [ ] Graceful no-op when `shift-ai` binary is not installed
- [ ] 51 runtime tests pass (`npx vitest run`)
