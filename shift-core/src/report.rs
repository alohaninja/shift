use serde::{Deserialize, Serialize};
use std::fmt;

use crate::cost::{ImageMetrics, TokenSavings};

/// Record of a single transformation action taken.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Which image (by index in the payload)
    pub image_index: usize,
    /// What action was taken
    pub action: String,
    /// Details (e.g., "resized from 4000x3000 to 2048x1536")
    pub detail: String,
}

/// Report of all transformations applied by SHIFT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Total payload size before transformation (bytes)
    pub original_size: usize,
    /// Total payload size after transformation (bytes)
    pub transformed_size: usize,
    /// Number of images found in the payload
    pub images_found: usize,
    /// Number of images that were modified
    pub images_modified: usize,
    /// Number of images dropped
    pub images_dropped: usize,
    /// Number of SVGs rasterized
    pub svgs_rasterized: usize,
    /// Individual action records
    pub actions: Vec<ActionRecord>,
    /// Warnings (non-fatal issues)
    pub warnings: Vec<String>,
    /// Whether this was a dry run (no actual changes)
    pub dry_run: bool,
    /// Per-image before/after metrics (dimensions, bytes, tokens)
    pub image_metrics: Vec<ImageMetrics>,
    /// Aggregate token savings across all images
    pub token_savings: TokenSavings,
}

impl Report {
    pub fn new() -> Self {
        Report {
            original_size: 0,
            transformed_size: 0,
            images_found: 0,
            images_modified: 0,
            images_dropped: 0,
            svgs_rasterized: 0,
            actions: Vec::new(),
            warnings: Vec::new(),
            dry_run: false,
            image_metrics: Vec::new(),
            token_savings: TokenSavings::default(),
        }
    }

    pub fn add_action(&mut self, image_index: usize, action: &str, detail: &str) {
        self.actions.push(ActionRecord {
            image_index,
            action: action.to_string(),
            detail: detail.to_string(),
        });
    }

    pub fn add_warning(&mut self, warning: &str) {
        self.warnings.push(warning.to_string());
    }

    pub fn add_image_metrics(&mut self, metrics: ImageMetrics) {
        self.image_metrics.push(metrics);
    }

    /// Recompute aggregate token savings from per-image metrics.
    pub fn finalize_token_savings(&mut self) {
        self.token_savings = TokenSavings::from_metrics(&self.image_metrics);
    }

    /// Size reduction as a percentage.
    pub fn size_reduction_pct(&self) -> f64 {
        if self.original_size == 0 {
            return 0.0;
        }
        let reduction = self.original_size as f64 - self.transformed_size as f64;
        (reduction / self.original_size as f64) * 100.0
    }

    /// Whether any transformations were actually applied.
    pub fn has_changes(&self) -> bool {
        self.images_modified > 0 || self.images_dropped > 0 || self.svgs_rasterized > 0
    }
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a token count with thousands separators.
fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dry_run {
            writeln!(f, "=== SHIFT Dry Run Report ===")?;
        } else {
            writeln!(f, "=== SHIFT Report ===")?;
        }

        writeln!(f, "Images found:      {}", self.images_found)?;
        writeln!(f, "Images modified:   {}", self.images_modified)?;
        writeln!(f, "Images dropped:    {}", self.images_dropped)?;
        if self.svgs_rasterized > 0 {
            writeln!(f, "SVGs rasterized:   {}", self.svgs_rasterized)?;
        }
        writeln!(f, "Original size:     {} bytes", self.original_size)?;
        writeln!(f, "Transformed size:  {} bytes", self.transformed_size)?;
        if self.original_size > 0 {
            writeln!(f, "Size reduction:    {:.1}%", self.size_reduction_pct())?;
        }

        // Token savings section
        let ts = &self.token_savings;
        if ts.openai_before > 0 || ts.anthropic_before > 0 {
            writeln!(f)?;
            writeln!(f, "Token Savings (estimated):")?;
            if ts.openai_before > 0 {
                writeln!(
                    f,
                    "  OpenAI:    {} -> {} tokens  ({:.1}%)",
                    fmt_tokens(ts.openai_before),
                    fmt_tokens(ts.openai_after),
                    -ts.openai_pct()
                )?;
            }
            if ts.anthropic_before > 0 {
                writeln!(
                    f,
                    "  Anthropic: {} -> {} tokens  ({:.1}%)",
                    fmt_tokens(ts.anthropic_before),
                    fmt_tokens(ts.anthropic_after),
                    -ts.anthropic_pct()
                )?;
            }
        }

        // Per-image breakdown
        if !self.image_metrics.is_empty()
            && self.image_metrics.iter().any(|m| {
                m.original_width != m.transformed_width || m.original_height != m.transformed_height
            })
        {
            writeln!(f)?;
            writeln!(f, "Per-image breakdown:")?;
            for m in &self.image_metrics {
                let dims_changed = m.original_width != m.transformed_width
                    || m.original_height != m.transformed_height;
                let fmt_changed = m.format_before != m.format_after;

                if dims_changed || fmt_changed {
                    let fmt_str = if fmt_changed {
                        format!(
                            "  {}->{}",
                            m.format_before.to_uppercase(),
                            m.format_after.to_uppercase()
                        )
                    } else {
                        String::new()
                    };
                    writeln!(
                        f,
                        "  [{}] {}x{} -> {}x{}{}  (OpenAI: {} -> {}, Anthropic: {} -> {})",
                        m.image_index,
                        m.original_width,
                        m.original_height,
                        m.transformed_width,
                        m.transformed_height,
                        fmt_str,
                        fmt_tokens(m.tokens_before.openai_tokens),
                        fmt_tokens(m.tokens_after.openai_tokens),
                        fmt_tokens(m.tokens_before.anthropic_tokens),
                        fmt_tokens(m.tokens_after.anthropic_tokens),
                    )?;
                }
            }
        }

        if !self.actions.is_empty() {
            writeln!(f, "\nActions:")?;
            for action in &self.actions {
                writeln!(
                    f,
                    "  [image {}] {} — {}",
                    action.image_index, action.action, action.detail
                )?;
            }
        }

        if !self.warnings.is_empty() {
            writeln!(f, "\nWarnings:")?;
            for warning in &self.warnings {
                writeln!(f, "  ! {}", warning)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_new() {
        let report = Report::new();
        assert_eq!(report.images_found, 0);
        assert!(!report.has_changes());
    }

    #[test]
    fn test_report_size_reduction() {
        let mut report = Report::new();
        report.original_size = 1000;
        report.transformed_size = 750;
        assert!((report.size_reduction_pct() - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_report_size_reduction_zero() {
        let report = Report::new();
        assert_eq!(report.size_reduction_pct(), 0.0);
    }

    #[test]
    fn test_report_has_changes() {
        let mut report = Report::new();
        assert!(!report.has_changes());

        report.images_modified = 1;
        assert!(report.has_changes());
    }

    #[test]
    fn test_report_display() {
        let mut report = Report::new();
        report.images_found = 2;
        report.images_modified = 1;
        report.original_size = 5000;
        report.transformed_size = 3000;
        report.add_action(0, "resize", "from 4000x3000 to 2048x1536");
        report.add_warning("image 1 is very small, may lose detail");

        let output = format!("{}", report);
        assert!(output.contains("Images found:      2"));
        assert!(output.contains("Images modified:   1"));
        assert!(output.contains("resize"));
        assert!(output.contains("may lose detail"));
    }

    #[test]
    fn test_report_display_with_token_savings() {
        use crate::cost::{estimate_tokens, ImageMetrics};

        let mut report = Report::new();
        report.images_found = 1;
        report.images_modified = 1;
        report.original_size = 5_000_000;
        report.transformed_size = 500_000;

        let before = estimate_tokens(4000, 3000);
        let after = estimate_tokens(2048, 1536);
        report.add_image_metrics(ImageMetrics {
            image_index: 0,
            original_width: 4000,
            original_height: 3000,
            transformed_width: 2048,
            transformed_height: 1536,
            original_bytes: 5_000_000,
            transformed_bytes: 500_000,
            format_before: "png".to_string(),
            format_after: "png".to_string(),
            tokens_before: before,
            tokens_after: after,
        });
        report.finalize_token_savings();

        let output = format!("{}", report);
        assert!(output.contains("Token Savings"));
        assert!(output.contains("OpenAI:"));
        assert!(output.contains("Anthropic:"));
        assert!(output.contains("Per-image breakdown:"));
    }

    #[test]
    fn test_fmt_tokens() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(42), "42");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1000), "1,000");
        assert_eq!(fmt_tokens(12345), "12,345");
        assert_eq!(fmt_tokens(1234567), "1,234,567");
    }
}
