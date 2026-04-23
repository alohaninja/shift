# shift-ai Preflight

Inspect and optimize base64-encoded images in AI API request payloads before sending. Helps prevent oversized-image failures and can reduce multimodal token usage.

## When to use this skill

Use this skill when you are:
- Building a JSON request payload that contains inline base64-encoded images for OpenAI or Anthropic
- Sending image-heavy payloads to `api.openai.com` or `api.anthropic.com`
- Reviewing code that constructs multimodal API requests with user-supplied images
- Troubleshooting 400 errors related to oversized images, invalid image payloads, or unsupported formats

Do not use this skill when:
- The payload contains only text
- Images are referenced only by URL
- The payload has already been optimized by `shift-ai`
- You need exact preservation of original image bytes

## Important behavior

- `shift-ai preflight` inspects the payload and reports what would change. It does **not** modify the input payload.
- `shift-ai` (without `preflight`) transforms the payload and writes optimized JSON to stdout.
- v1 supports **inline base64-encoded images only**. URL-referenced images may be detected but are not transformed.
- Non-zero exit code on malformed JSON, unsupported provider, or unreadable input.
- Token estimates are approximations based on published provider formulas. Actual billing may differ by model.

## Step 1: Run preflight

Write the payload to a file, then run:

```bash
shift-ai preflight /tmp/payload.json --provider <openai|anthropic> --mode balanced
```

Or pipe from stdin:

```bash
cat /tmp/payload.json | shift-ai preflight --provider anthropic
```

The output is structured JSON. Key fields:

- `images_found` -- total images detected in the payload
- `images_needing_transform` -- images that exceed provider constraints or would benefit from optimization
- `images_ok` -- images already within constraints
- `token_estimate` -- estimated tokens before/after for both OpenAI and Anthropic
- `recommendations` -- actionable suggestions (e.g., switch to economy mode for more savings)
- `api_key_present` -- whether the provider's API key env var is set
- `warnings` -- any issues detected (format conversions, unsupported types)

## Step 2: Decide whether to optimize

Apply this decision logic in order:

1. **Safety first**: if any image exceeds provider hard limits (max dimension, max file size, unsupported format), always optimize. The request will fail without it.
2. **Cost savings**: if `images_needing_transform > 0` and estimated token savings exceed 10%, optimize.
3. **User intent**: if the user explicitly requested optimization or cost reduction, optimize regardless of threshold.
4. **Skip**: if `images_found` is 0 or `images_needing_transform` is 0, no optimization needed.

## Step 3: Apply optimization

Run shift-ai to transform the payload:

```bash
shift-ai /tmp/payload.json --provider <provider> --mode <mode> > /tmp/optimized.json
```

Mode selection:
- `balanced` (default) -- moderate optimization, preserves quality. Good for most use cases.
- `economy` -- aggressive downscaling to 1024px, maximizes token savings. Use when cost matters more than image detail.
- `performance` -- minimal transforms, only enforces hard provider limits. Use when image quality is critical.

## Step 4: Send the optimized payload

Use the optimized payload for the API call:

```bash
curl -X POST https://api.anthropic.com/v1/messages \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d @/tmp/optimized.json
```

## Handling failures

If `shift-ai preflight` or `shift-ai` exits non-zero:

- **Malformed JSON** -- fix the payload structure and retry
- **No images found** -- no optimization possible; send the payload as-is
- **Unsupported image format** -- the image type is not handled by v1; send as-is or convert manually
- **Missing API key warning** (in preflight report, not a failure) -- set the appropriate env var before sending

When in doubt, send the original payload unmodified. shift-ai is an optimization, not a requirement.

## Example workflow

```bash
# 1. Preflight check
cat request.json | shift-ai preflight -p anthropic -m economy
# Look at images_needing_transform and token_estimate in the output

# 2. Optimize (if warranted)
shift-ai request.json -p anthropic -m economy > safe_request.json

# 3. Send
curl -X POST https://api.anthropic.com/v1/messages \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d @safe_request.json
```

## Cumulative savings tracking

shift-ai records run statistics automatically. View cumulative savings:

```bash
shift-ai gain              # summary
shift-ai gain --daily      # day-by-day breakdown
shift-ai gain --format json  # machine-readable
```
