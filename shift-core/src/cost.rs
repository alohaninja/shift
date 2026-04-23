//! Token cost estimation for AI vision providers.
//!
//! Both OpenAI and Anthropic charge tokens for image inputs based on
//! image dimensions. This module implements the public token-counting
//! formulas so SHIFT can report estimated savings.

use serde::{Deserialize, Serialize};

/// Estimated token counts for a single image across providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenEstimate {
    pub openai_tokens: u64,
    pub anthropic_tokens: u64,
}

/// Per-image before/after metrics for the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetrics {
    /// Index in the original payload
    pub image_index: usize,
    /// Original dimensions
    pub original_width: u32,
    pub original_height: u32,
    /// Transformed dimensions (same as original if unchanged)
    pub transformed_width: u32,
    pub transformed_height: u32,
    /// Original byte size of the raw image
    pub original_bytes: usize,
    /// Transformed byte size
    pub transformed_bytes: usize,
    /// Format before transformation (e.g. "png", "jpeg", "svg")
    pub format_before: String,
    /// Format after transformation
    pub format_after: String,
    /// Estimated tokens before transformation
    pub tokens_before: TokenEstimate,
    /// Estimated tokens after transformation
    pub tokens_after: TokenEstimate,
}

/// Aggregate token savings across all images.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenSavings {
    pub openai_before: u64,
    pub openai_after: u64,
    pub anthropic_before: u64,
    pub anthropic_after: u64,
}

impl TokenSavings {
    pub fn openai_saved(&self) -> u64 {
        self.openai_before.saturating_sub(self.openai_after)
    }

    pub fn anthropic_saved(&self) -> u64 {
        self.anthropic_before.saturating_sub(self.anthropic_after)
    }

    pub fn openai_pct(&self) -> f64 {
        if self.openai_before == 0 {
            return 0.0;
        }
        (self.openai_saved() as f64 / self.openai_before as f64) * 100.0
    }

    pub fn anthropic_pct(&self) -> f64 {
        if self.anthropic_before == 0 {
            return 0.0;
        }
        (self.anthropic_saved() as f64 / self.anthropic_before as f64) * 100.0
    }

    /// Aggregate from per-image metrics.
    ///
    /// Excludes dropped images (transformed dimensions 0×0 with non-zero
    /// original dimensions) so that information removal is not counted as
    /// token "savings". Use [`from_metrics_all`] if you want raw totals.
    pub fn from_metrics(metrics: &[ImageMetrics]) -> Self {
        let mut s = TokenSavings::default();
        for m in metrics {
            // Skip dropped images: original had tokens but transformed is 0×0
            let was_dropped = (m.original_width > 0 || m.original_height > 0)
                && m.transformed_width == 0
                && m.transformed_height == 0;
            if was_dropped {
                continue;
            }
            s.openai_before += m.tokens_before.openai_tokens;
            s.openai_after += m.tokens_after.openai_tokens;
            s.anthropic_before += m.tokens_before.anthropic_tokens;
            s.anthropic_after += m.tokens_after.anthropic_tokens;
        }
        s
    }

    /// Aggregate from all per-image metrics including dropped images.
    pub fn from_metrics_all(metrics: &[ImageMetrics]) -> Self {
        let mut s = TokenSavings::default();
        for m in metrics {
            s.openai_before += m.tokens_before.openai_tokens;
            s.openai_after += m.tokens_after.openai_tokens;
            s.anthropic_before += m.tokens_before.anthropic_tokens;
            s.anthropic_after += m.tokens_after.anthropic_tokens;
        }
        s
    }
}

// ── OpenAI token estimation (tile-based, GPT-4o / GPT-4.1 family) ───

/// OpenAI vision token count for `detail: high`.
///
/// Algorithm (from OpenAI docs, tile-based family):
/// 1. Scale image so shortest side = 768px (only if larger)
/// 2. Split into 512×512 tiles (ceiling)
/// 3. Each tile = 170 tokens + 85 base tokens
///
/// For `detail: low`: fixed 85 tokens.
///
/// **Accuracy:** This implements the tile-based formula that is correct for
/// **GPT-4o, GPT-4.1, and GPT-4.5** (base=85, tile=170). Other model
/// families use different constants:
///
/// | Model family          | Base | Tile   | This function |
/// |-----------------------|------|--------|---------------|
/// | GPT-4o / 4.1 / 4.5   |   85 |   170  | Correct       |
/// | GPT-4o-mini           | 2833 | 5,667  | ~33× under    |
/// | o1 / o1-pro / o3      |   75 |   150  | ~13% over     |
///
/// Newer models (GPT-4.1 2025-04-14+, o4-mini) use patch-based tokenization
/// with different budgets and are not covered by this formula.
pub fn openai_tokens(width: u32, height: u32) -> u64 {
    if width == 0 || height == 0 {
        return 0;
    }

    // detail: high calculation
    let (w, h) = openai_scale_to_fit(width, height);
    let tiles_w = (w as f64 / 512.0).ceil() as u64;
    let tiles_h = (h as f64 / 512.0).ceil() as u64;
    let tiles = tiles_w * tiles_h;

    170 * tiles + 85
}

/// Scale so the shortest side is at most 768px, preserving aspect ratio.
/// Also cap the longest side at 2048px (OpenAI constraint).
fn openai_scale_to_fit(width: u32, height: u32) -> (u32, u32) {
    let mut w = width as f64;
    let mut h = height as f64;

    // Cap longest side at 2048
    let max_dim = w.max(h);
    if max_dim > 2048.0 {
        let scale = 2048.0 / max_dim;
        w *= scale;
        h *= scale;
    }

    // Scale so shortest side is at most 768
    let min_side = w.min(h);
    if min_side > 768.0 {
        let scale = 768.0 / min_side;
        w *= scale;
        h *= scale;
    }

    (w.ceil() as u32, h.ceil() as u32)
}

/// Fixed token count for OpenAI detail: low.
pub fn openai_tokens_low() -> u64 {
    85
}

// ── Anthropic token estimation ───────────────────────────────────────

/// Anthropic vision token count for standard-resolution models.
///
/// Formula (from Anthropic docs):
///   tokens ≈ (width × height) / 750
///
/// Images are first downscaled so the long edge ≤ 1568px (standard models)
/// and then padded to a multiple of 28px.
/// Max tokens per image: 1568 (standard) or 4784 (Opus 4.7).
///
/// **Note:** This implements the standard-resolution formula (1568px max
/// long edge, 1568 token cap). Claude Opus 4.7 supports high-resolution
/// images (2576px long edge, 4784 token cap). Estimates for Opus 4.7
/// payloads with large images will be under-counted.
pub fn anthropic_tokens(width: u32, height: u32) -> u64 {
    if width == 0 || height == 0 {
        return 0;
    }

    let (w, h) = anthropic_scale_to_fit(width, height);

    // Pad to next multiple of 28
    let pw = next_multiple_of_28(w);
    let ph = next_multiple_of_28(h);

    let tokens = (pw as u64 * ph as u64) / 750;
    // Cap at 1568 tokens (standard models)
    tokens.min(1568)
}

/// Scale so the long edge is at most 1568px, preserving aspect ratio.
fn anthropic_scale_to_fit(width: u32, height: u32) -> (u32, u32) {
    let max_edge = 1568.0_f64;
    let w = width as f64;
    let h = height as f64;
    let long_edge = w.max(h);

    if long_edge <= max_edge {
        return (width, height);
    }

    let scale = max_edge / long_edge;
    ((w * scale).ceil() as u32, (h * scale).ceil() as u32)
}

fn next_multiple_of_28(val: u32) -> u32 {
    val.div_ceil(28) * 28
}

/// Estimate tokens for an image at given dimensions.
pub fn estimate_tokens(width: u32, height: u32) -> TokenEstimate {
    TokenEstimate {
        openai_tokens: openai_tokens(width, height),
        anthropic_tokens: anthropic_tokens(width, height),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── OpenAI ───────────────────────────────────────────────────

    #[test]
    fn test_openai_small_image() {
        // 512x512: fits in one tile → 170 + 85 = 255
        assert_eq!(openai_tokens(512, 512), 255);
    }

    #[test]
    fn test_openai_768_image() {
        // 768x768: shortest side = 768, so no scaling needed.
        // Tiles: ceil(768/512) = 2 per side → 4 tiles → 4*170 + 85 = 765
        assert_eq!(openai_tokens(768, 768), 765);
    }

    #[test]
    fn test_openai_large_landscape() {
        // 4000x3000:
        // Step 1: cap longest at 2048 → scale = 2048/4000 = 0.512
        //   → 2048 x 1536
        // Step 2: shortest side 1536 > 768 → scale = 768/1536 = 0.5
        //   → 1024 x 768
        // Tiles: ceil(1024/512) * ceil(768/512) = 2 * 2 = 4
        // Tokens: 4 * 170 + 85 = 765
        assert_eq!(openai_tokens(4000, 3000), 765);
    }

    #[test]
    fn test_openai_tall_portrait() {
        // 1000x4000:
        // Step 1: cap longest at 2048 → scale = 2048/4000 = 0.512
        //   → 512 x 2048
        // Step 2: shortest side 512 ≤ 768 → no change
        // Tiles: ceil(512/512) * ceil(2048/512) = 1 * 4 = 4
        // Tokens: 4 * 170 + 85 = 765
        assert_eq!(openai_tokens(1000, 4000), 765);
    }

    #[test]
    fn test_openai_zero() {
        assert_eq!(openai_tokens(0, 0), 0);
    }

    #[test]
    fn test_openai_low_detail() {
        assert_eq!(openai_tokens_low(), 85);
    }

    #[test]
    fn test_openai_very_small() {
        // 100x100: fits in one tile
        assert_eq!(openai_tokens(100, 100), 255);
    }

    // ── Anthropic ────────────────────────────────────────────────

    #[test]
    fn test_anthropic_small_image() {
        // 200x200: no scaling, pad to 224x224
        // tokens = 224*224/750 = 66.9 → 66
        assert_eq!(anthropic_tokens(200, 200), 66);
    }

    #[test]
    fn test_anthropic_1000x1000() {
        // 1000x1000: long edge ≤ 1568, no scaling
        // pad to 1008x1008 (1000 → next mult of 28 = 1008)
        // tokens = 1008*1008/750 = 1354
        assert_eq!(anthropic_tokens(1000, 1000), 1354);
    }

    #[test]
    fn test_anthropic_large_downscaled() {
        // 3000x2000: long edge 3000 > 1568
        // scale = 1568/3000 = 0.5227 → 1568 x 1046 (ceil)
        // pad: 1568 (already mult of 28), 1046 → 1064
        // tokens = 1568*1064/750 = 2224, capped at 1568
        assert_eq!(anthropic_tokens(3000, 2000), 1568);
    }

    #[test]
    fn test_anthropic_zero() {
        assert_eq!(anthropic_tokens(0, 0), 0);
    }

    #[test]
    fn test_anthropic_exact_max() {
        // 1568x1568: at limit
        // pad: both already multiple of 28 (1568 = 56*28)
        // tokens = 1568*1568/750 = 3277, capped at 1568
        assert_eq!(anthropic_tokens(1568, 1568), 1568);
    }

    // ── TokenSavings ─────────────────────────────────────────────

    #[test]
    fn test_savings_calculation() {
        let s = TokenSavings {
            openai_before: 1000,
            openai_after: 300,
            anthropic_before: 2000,
            anthropic_after: 500,
        };
        assert_eq!(s.openai_saved(), 700);
        assert_eq!(s.anthropic_saved(), 1500);
        assert!((s.openai_pct() - 70.0).abs() < 0.1);
        assert!((s.anthropic_pct() - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_savings_zero_before() {
        let s = TokenSavings::default();
        assert_eq!(s.openai_pct(), 0.0);
        assert_eq!(s.anthropic_pct(), 0.0);
    }

    // ── estimate_tokens ──────────────────────────────────────────

    #[test]
    fn test_estimate_tokens_both() {
        let est = estimate_tokens(1000, 1000);
        assert!(est.openai_tokens > 0);
        assert!(est.anthropic_tokens > 0);
    }

    // ── estimate_tokens: different dimensions produce different results ─

    #[test]
    fn test_estimate_tokens_varies_by_size() {
        let small = estimate_tokens(100, 100);
        let large = estimate_tokens(4000, 3000);
        // A 100x100 and 4000x3000 should not produce identical estimates
        // (OpenAI: 255 vs 765; Anthropic: different too)
        assert_ne!(
            small.openai_tokens, large.openai_tokens,
            "different dimensions should produce different OpenAI estimates"
        );
    }

    // ── Extreme aspect ratios ────────────────────────────────────

    #[test]
    fn test_openai_extreme_tall() {
        // 1x10000:
        // Cap longest at 2048 → scale=2048/10000=0.2048 → ceil(1*0.2048)=1, ceil(10000*0.2048)=2048
        // Shortest side 1 ≤ 768 → no further scaling
        // Tiles: ceil(1/512)=1 * ceil(2048/512)=4 → 4 tiles → 4*170+85 = 765
        assert_eq!(openai_tokens(1, 10000), 765);
    }

    #[test]
    fn test_openai_extreme_wide() {
        // 10000x1: same logic, just rotated
        assert_eq!(openai_tokens(10000, 1), 765);
    }

    #[test]
    fn test_anthropic_extreme_tall() {
        // 1x10000: long edge 10000 > 1568 → scale=1568/10000=0.1568
        //   → ceil(1*0.1568)=1, ceil(10000*0.1568)=1568
        // pad: 1→28, 1568→1568
        // tokens = 28*1568/750 = 58
        let tokens = anthropic_tokens(1, 10000);
        assert!(tokens > 0 && tokens < 100, "got {}", tokens);
    }

    #[test]
    fn test_openai_1x1() {
        // 1x1: fits in one tile → 170 + 85 = 255
        assert_eq!(openai_tokens(1, 1), 255);
    }

    #[test]
    fn test_anthropic_1x1() {
        // 1x1: pad to 28x28, tokens = 28*28/750 = 1
        assert_eq!(anthropic_tokens(1, 1), 1);
    }

    // ── Scaling helpers ──────────────────────────────────────────

    #[test]
    fn test_next_multiple_of_28() {
        assert_eq!(next_multiple_of_28(28), 28);
        assert_eq!(next_multiple_of_28(29), 56);
        assert_eq!(next_multiple_of_28(1), 28);
        assert_eq!(next_multiple_of_28(200), 224);
        assert_eq!(next_multiple_of_28(1568), 1568);
    }

    // ── TokenSavings: dropped images excluded ────────────────────

    #[test]
    fn test_savings_excludes_dropped() {
        use crate::cost::ImageMetrics;

        let metrics = vec![
            // Normal resize: 4000x3000 -> 2048x1536
            ImageMetrics {
                image_index: 0,
                original_width: 4000,
                original_height: 3000,
                transformed_width: 2048,
                transformed_height: 1536,
                original_bytes: 5_000_000,
                transformed_bytes: 500_000,
                format_before: "png".to_string(),
                format_after: "png".to_string(),
                tokens_before: estimate_tokens(4000, 3000),
                tokens_after: estimate_tokens(2048, 1536),
            },
            // Dropped image: 1000x1000 -> 0x0
            ImageMetrics {
                image_index: 1,
                original_width: 1000,
                original_height: 1000,
                transformed_width: 0,
                transformed_height: 0,
                original_bytes: 100_000,
                transformed_bytes: 0,
                format_before: "png".to_string(),
                format_after: "png".to_string(),
                tokens_before: estimate_tokens(1000, 1000),
                tokens_after: estimate_tokens(0, 0),
            },
        ];

        let savings = TokenSavings::from_metrics(&metrics);
        let savings_all = TokenSavings::from_metrics_all(&metrics);

        // from_metrics should only include the resize, not the drop
        assert_eq!(
            savings.openai_before,
            estimate_tokens(4000, 3000).openai_tokens
        );
        assert_eq!(
            savings.openai_after,
            estimate_tokens(2048, 1536).openai_tokens
        );

        // from_metrics_all should include both
        assert_eq!(
            savings_all.openai_before,
            estimate_tokens(4000, 3000).openai_tokens + estimate_tokens(1000, 1000).openai_tokens
        );
    }
}
