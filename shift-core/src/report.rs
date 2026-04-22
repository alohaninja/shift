use serde::{Deserialize, Serialize};
use std::fmt;

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
}
