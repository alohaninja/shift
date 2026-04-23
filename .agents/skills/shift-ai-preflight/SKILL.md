# shift-ai Preflight

Optimize images in AI API request payloads before sending. Reduces Anthropic token cost by 30-60% and prevents oversized-image failures.

## When to use this skill

Activate when you are:
- Building a JSON request payload that contains base64-encoded images for OpenAI or Anthropic
- Piping image-heavy content to `curl` calls against `api.openai.com` or `api.anthropic.com`
- Reviewing code that constructs multimodal API requests with user-supplied images
- Troubleshooting 400 errors from oversized images or unsupported formats

Do NOT use when:
- The payload contains only text (no images)
- Images are referenced by URL (not base64-encoded in the payload)
- The request has already been processed by shift-ai

## Step 1: Run preflight

Write the payload to a temp file, then run preflight to assess optimization potential:

```bash
shift-ai preflight /tmp/payload.json --provider <openai|anthropic> --mode balanced
```

Or pipe from stdin:

```bash
cat /tmp/payload.json | shift-ai preflight --provider anthropic
```

The output is structured JSON with these key fields:

| Field | Meaning |
|-------|---------|
| `images_found` | Total images detected in the payload |
| `images_needing_transform` | Images that exceed provider constraints or would benefit from optimization |
| `images_ok` | Images already within constraints |
| `token_estimate.anthropic_before` / `anthropic_after` | Estimated Anthropic tokens before/after optimization |
| `token_estimate.openai_before` / `openai_after` | Estimated OpenAI tokens before/after optimization |
| `recommendations` | Actionable suggestions (e.g., switch to economy mode) |
| `api_key_present` | Whether the provider's API key env var is set |

## Step 2: Decide whether to optimize

Apply this decision logic based on the preflight report:

1. **If `images_found` is 0** -- skip, no optimization needed
2. **If `images_needing_transform` is 0** -- skip, all images within constraints
3. **If Anthropic savings > 10%** -- optimize (Anthropic charges per-pixel, savings are significant)
4. **If OpenAI savings > 10%** -- optimize
5. **If savings < 10%** -- skip unless the user explicitly requested optimization

## Step 3: Apply optimization

If optimization is warranted, run shift-ai to transform the payload:

```bash
shift-ai /tmp/payload.json --provider <provider> --mode <mode> > /tmp/optimized.json
```

Mode selection:
- **`balanced`** (default) -- moderate optimization, preserves quality. Good for most use cases.
- **`economy`** -- aggressive downscaling to 1024px, maximizes token savings. Use when cost matters more than image detail.
- **`performance`** -- minimal transforms, only enforces hard provider limits. Use when image quality is critical.

## Step 4: Send the optimized payload

Use the optimized payload for the API call:

```bash
curl -X POST https://api.anthropic.com/v1/messages \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d @/tmp/optimized.json
```

## Example workflow

```bash
# 1. Preflight check
cat request.json | shift-ai preflight -p anthropic -m economy
# Output: images_needing_transform: 3, Anthropic savings: 45%

# 2. Optimize
shift-ai request.json -p anthropic -m economy > safe_request.json

# 3. Send
curl -X POST https://api.anthropic.com/v1/messages \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d @safe_request.json
```

## Provider token costs

Understanding why optimization matters:

| Provider | Token formula | Key constraint |
|----------|-------------|----------------|
| **Anthropic** | `width * height / 750` tokens per image | 1568px long-edge cap (standard models). Larger images are auto-resized server-side but you pay bandwidth. |
| **OpenAI** | Tile-based: 512x512 tiles, `170 * tiles + 85` tokens | 2048px max dimension. `detail: "low"` forces 512x512 at 85 tokens. |

Economy mode targets 1024px long-edge, which yields ~31% Anthropic savings on typical 4000x3000 images.

## Cumulative savings tracking

shift-ai automatically records run statistics. View cumulative savings:

```bash
shift-ai gain              # summary
shift-ai gain --daily      # day-by-day breakdown
shift-ai gain --format json  # machine-readable
```
