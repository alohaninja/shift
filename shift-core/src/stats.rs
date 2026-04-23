//! Persistent run statistics for cumulative token savings tracking.
//!
//! Stores one JSON line per SHIFT invocation in `~/.shift/stats.jsonl`.
//! Inspired by RTK's `rtk gain` analytics system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::cost::TokenSavings;

/// A single run record persisted to the stats file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Date portion (YYYY-MM-DD) for daily aggregation
    pub date: String,
    /// Provider used
    pub provider: String,
    /// Number of images processed
    pub images: usize,
    /// Number of images modified
    pub modified: usize,
    /// Byte sizes
    pub bytes_before: usize,
    pub bytes_after: usize,
    /// Token savings
    pub token_savings: TokenSavings,
}

/// Aggregated gain summary.
#[derive(Debug, Clone, Default)]
pub struct GainSummary {
    pub total_runs: usize,
    pub total_images: usize,
    pub total_modified: usize,
    pub total_bytes_before: u64,
    pub total_bytes_after: u64,
    pub total_openai_before: u64,
    pub total_openai_after: u64,
    pub total_anthropic_before: u64,
    pub total_anthropic_after: u64,
}

/// Daily aggregation bucket.
#[derive(Debug, Clone)]
pub struct DailyGain {
    pub date: String,
    pub runs: usize,
    pub images: usize,
    pub openai_saved: u64,
    pub anthropic_saved: u64,
}

impl GainSummary {
    pub fn openai_saved(&self) -> u64 {
        self.total_openai_before
            .saturating_sub(self.total_openai_after)
    }

    pub fn anthropic_saved(&self) -> u64 {
        self.total_anthropic_before
            .saturating_sub(self.total_anthropic_after)
    }

    pub fn openai_pct(&self) -> f64 {
        if self.total_openai_before == 0 {
            return 0.0;
        }
        (self.openai_saved() as f64 / self.total_openai_before as f64) * 100.0
    }

    pub fn anthropic_pct(&self) -> f64 {
        if self.total_anthropic_before == 0 {
            return 0.0;
        }
        (self.anthropic_saved() as f64 / self.total_anthropic_before as f64) * 100.0
    }

    pub fn bytes_saved(&self) -> u64 {
        self.total_bytes_before
            .saturating_sub(self.total_bytes_after)
    }
}

/// Get the default stats file path: `~/.shift/stats.jsonl`.
pub fn default_stats_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("could not determine home directory")?;
    Ok(PathBuf::from(home).join(".shift").join("stats.jsonl"))
}

/// Append a run record to the stats file.
pub fn record_run(record: &RunRecord, path: Option<&PathBuf>) -> Result<()> {
    let stats_path = match path {
        Some(p) => p.clone(),
        None => default_stats_path()?,
    };

    // Ensure parent directory exists
    if let Some(parent) = stats_path.parent() {
        fs::create_dir_all(parent).context("failed to create ~/.shift directory")?;

        // Reject symlinks on the directory (consistent with pipeline.rs profile path validation)
        let dir_meta = fs::symlink_metadata(parent)
            .with_context(|| format!("failed to stat {}", parent.display()))?;
        if dir_meta.file_type().is_symlink() {
            anyhow::bail!(
                "stats directory {} is a symlink (possible symlink attack)",
                parent.display()
            );
        }
    }

    // Reject symlinks on the stats file itself
    if stats_path.exists() {
        let file_meta = fs::symlink_metadata(&stats_path)
            .with_context(|| format!("failed to stat {}", stats_path.display()))?;
        if file_meta.file_type().is_symlink() {
            anyhow::bail!(
                "stats file {} is a symlink (possible symlink attack)",
                stats_path.display()
            );
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stats_path)
        .with_context(|| format!("failed to open stats file: {}", stats_path.display()))?;

    // Serialize to a single buffer and write atomically to reduce interleave risk
    let mut line = serde_json::to_string(record).context("failed to serialize run record")?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .context("failed to write to stats file")?;
    file.flush().context("failed to flush stats file")?;

    Ok(())
}

/// Result of loading stats records, including count of skipped malformed lines.
pub struct LoadResult {
    pub records: Vec<RunRecord>,
    pub skipped_lines: usize,
}

/// Load all run records from the stats file.
pub fn load_records(path: Option<&PathBuf>) -> Result<LoadResult> {
    let stats_path = match path {
        Some(p) => p.clone(),
        None => default_stats_path()?,
    };

    if !stats_path.exists() {
        return Ok(LoadResult {
            records: Vec::new(),
            skipped_lines: 0,
        });
    }

    let file = fs::File::open(&stats_path)
        .with_context(|| format!("failed to open stats file: {}", stats_path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut skipped_lines = 0;

    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read line {} of stats file", i + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<RunRecord>(trimmed) {
            Ok(record) => records.push(record),
            Err(e) => {
                // Skip malformed lines rather than failing
                eprintln!(
                    "shift: warning: skipping malformed stats line {}: {}",
                    i + 1,
                    e
                );
                skipped_lines += 1;
            }
        }
    }

    Ok(LoadResult {
        records,
        skipped_lines,
    })
}

/// Compute aggregate gain summary from records.
pub fn summarize(records: &[RunRecord]) -> GainSummary {
    let mut s = GainSummary::default();
    for r in records {
        s.total_runs += 1;
        s.total_images += r.images;
        s.total_modified += r.modified;
        s.total_bytes_before += r.bytes_before as u64;
        s.total_bytes_after += r.bytes_after as u64;
        s.total_openai_before += r.token_savings.openai_before;
        s.total_openai_after += r.token_savings.openai_after;
        s.total_anthropic_before += r.token_savings.anthropic_before;
        s.total_anthropic_after += r.token_savings.anthropic_after;
    }
    s
}

/// Compute daily breakdown from records.
pub fn daily_breakdown(records: &[RunRecord]) -> Vec<DailyGain> {
    use std::collections::BTreeMap;

    let mut days: BTreeMap<String, DailyGain> = BTreeMap::new();

    for r in records {
        let entry = days.entry(r.date.clone()).or_insert_with(|| DailyGain {
            date: r.date.clone(),
            runs: 0,
            images: 0,
            openai_saved: 0,
            anthropic_saved: 0,
        });
        entry.runs += 1;
        entry.images += r.images;
        entry.openai_saved += r
            .token_savings
            .openai_before
            .saturating_sub(r.token_savings.openai_after);
        entry.anthropic_saved += r
            .token_savings
            .anthropic_before
            .saturating_sub(r.token_savings.anthropic_after);
    }

    days.into_values().collect()
}

/// Build a RunRecord from a completed Report.
pub fn record_from_report(report: &crate::report::Report, provider: &str) -> RunRecord {
    // Get current timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Format as ISO 8601 (basic — no chrono dependency)
    let secs_per_day = 86400;
    let days_since_epoch = now / secs_per_day;
    let secs_today = now % secs_per_day;
    let hours = secs_today / 3600;
    let minutes = (secs_today % 3600) / 60;
    let seconds = secs_today % 60;

    // Simple date calculation (approximate, good enough for stats)
    let (year, month, day) = days_to_ymd(days_since_epoch);

    let timestamp = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    );
    let date = format!("{:04}-{:02}-{:02}", year, month, day);

    RunRecord {
        timestamp,
        date,
        provider: provider.to_string(),
        images: report.images_found,
        modified: report.images_modified,
        bytes_before: report.original_size,
        bytes_after: report.transformed_size,
        token_savings: report.token_savings.clone(),
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified civil date calculation
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::TokenSavings;
    use tempfile::NamedTempFile;

    fn make_record(date: &str, openai_before: u64, openai_after: u64) -> RunRecord {
        RunRecord {
            timestamp: format!("{}T12:00:00Z", date),
            date: date.to_string(),
            provider: "openai".to_string(),
            images: 3,
            modified: 2,
            bytes_before: 5_000_000,
            bytes_after: 1_000_000,
            token_savings: TokenSavings {
                openai_before,
                openai_after,
                anthropic_before: 3000,
                anthropic_after: 1000,
            },
        }
    }

    #[test]
    fn test_record_and_load_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let r1 = make_record("2026-04-20", 1000, 300);
        let r2 = make_record("2026-04-21", 2000, 500);

        record_run(&r1, Some(&path)).unwrap();
        record_run(&r2, Some(&path)).unwrap();

        let result = load_records(Some(&path)).unwrap();
        assert_eq!(result.records.len(), 2);
        assert_eq!(result.skipped_lines, 0);
        assert_eq!(result.records[0].date, "2026-04-20");
        assert_eq!(result.records[1].date, "2026-04-21");
    }

    #[test]
    fn test_load_empty_file() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let result = load_records(Some(&path)).unwrap();
        assert!(result.records.is_empty());
        assert_eq!(result.skipped_lines, 0);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let path = PathBuf::from("/tmp/shift-test-nonexistent-stats.jsonl");
        let result = load_records(Some(&path)).unwrap();
        assert!(result.records.is_empty());
        assert_eq!(result.skipped_lines, 0);
    }

    #[test]
    fn test_summarize() {
        let records = vec![
            make_record("2026-04-20", 1000, 300),
            make_record("2026-04-21", 2000, 500),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.total_runs, 2);
        assert_eq!(summary.total_images, 6);
        assert_eq!(summary.total_modified, 4);
        assert_eq!(summary.total_openai_before, 3000);
        assert_eq!(summary.total_openai_after, 800);
        assert_eq!(summary.openai_saved(), 2200);
    }

    #[test]
    fn test_daily_breakdown() {
        let records = vec![
            make_record("2026-04-20", 1000, 300),
            make_record("2026-04-20", 500, 200),
            make_record("2026-04-21", 2000, 500),
        ];
        let daily = daily_breakdown(&records);
        assert_eq!(daily.len(), 2);
        assert_eq!(daily[0].date, "2026-04-20");
        assert_eq!(daily[0].runs, 2);
        assert_eq!(daily[0].openai_saved, 1000); // (1000-300) + (500-200)
        assert_eq!(daily[1].date, "2026-04-21");
        assert_eq!(daily[1].runs, 1);
    }

    #[test]
    fn test_summary_percentages() {
        let summary = GainSummary {
            total_openai_before: 10000,
            total_openai_after: 3000,
            total_anthropic_before: 5000,
            total_anthropic_after: 1000,
            ..Default::default()
        };
        assert!((summary.openai_pct() - 70.0).abs() < 0.1);
        assert!((summary.anthropic_pct() - 80.0).abs() < 0.1);
    }

    #[test]
    fn test_summary_zero_division() {
        let summary = GainSummary::default();
        assert_eq!(summary.openai_pct(), 0.0);
        assert_eq!(summary.anthropic_pct(), 0.0);
    }

    #[test]
    fn test_malformed_lines_skipped() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Write valid + invalid lines
        let r = make_record("2026-04-20", 1000, 300);
        record_run(&r, Some(&path)).unwrap();
        // Append garbage
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "not json at all").unwrap();
        writeln!(f, "{{\"partial\": true}}").unwrap();
        // Write another valid record
        record_run(&r, Some(&path)).unwrap();

        let result = load_records(Some(&path)).unwrap();
        assert_eq!(result.records.len(), 2); // only the 2 valid records
        assert_eq!(result.skipped_lines, 2); // 2 malformed lines skipped
    }

    #[test]
    fn test_record_from_report() {
        let mut report = crate::report::Report::new();
        report.images_found = 3;
        report.images_modified = 2;
        report.original_size = 5_000_000;
        report.transformed_size = 1_000_000;
        report.token_savings = TokenSavings {
            openai_before: 2000,
            openai_after: 500,
            anthropic_before: 3000,
            anthropic_after: 800,
        };

        let record = record_from_report(&report, "openai");
        assert_eq!(record.provider, "openai");
        assert_eq!(record.images, 3);
        assert_eq!(record.modified, 2);
        assert!(!record.timestamp.is_empty());
        assert!(!record.date.is_empty());
    }

    #[test]
    fn test_days_to_ymd() {
        // Unix epoch
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));

        // Leap year: 2000-02-29 = day 11016
        let (y, m, d) = days_to_ymd(11016);
        assert_eq!((y, m, d), (2000, 2, 29));

        // Day after leap day: 2000-03-01 = day 11017
        let (y, m, d) = days_to_ymd(11017);
        assert_eq!((y, m, d), (2000, 3, 1));

        // Non-leap century year: 2100-02-28 = day 47540
        let (y, m, d) = days_to_ymd(47540);
        assert_eq!((y, m, d), (2100, 2, 28));

        // 2100-03-01 = day 47541 (no Feb 29 in 2100)
        let (y, m, d) = days_to_ymd(47541);
        assert_eq!((y, m, d), (2100, 3, 1));

        // Year boundary: 2025-12-31 = day 20453
        let (y, m, d) = days_to_ymd(20453);
        assert_eq!((y, m, d), (2025, 12, 31));

        // 2026-01-01 = day 20454
        let (y, m, d) = days_to_ymd(20454);
        assert_eq!((y, m, d), (2026, 1, 1));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_directory_rejected() {
        use std::os::unix::fs as unix_fs;

        let real_dir = tempfile::tempdir().unwrap();
        let symlink_dir = tempfile::tempdir().unwrap();
        let symlink_path = symlink_dir.path().join("symlinked-shift");

        // Create symlink to real directory
        unix_fs::symlink(real_dir.path(), &symlink_path).unwrap();

        let stats_file = symlink_path.join("stats.jsonl");
        let r = make_record("2026-04-22", 100, 50);
        let result = record_run(&r, Some(&stats_file));

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("symlink"),
            "expected symlink error, got: {}",
            err_msg
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_file_rejected() {
        use std::os::unix::fs as unix_fs;

        let tmp_dir = tempfile::tempdir().unwrap();
        let real_file = tmp_dir.path().join("real-stats.jsonl");
        let symlink_file = tmp_dir.path().join("stats.jsonl");

        // Create the real file
        fs::write(&real_file, "").unwrap();
        // Create symlink pointing to real file
        unix_fs::symlink(&real_file, &symlink_file).unwrap();

        let r = make_record("2026-04-22", 100, 50);
        let result = record_run(&r, Some(&symlink_file));

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("symlink"),
            "expected symlink error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_skipped_lines_counted() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let r = make_record("2026-04-22", 500, 200);
        record_run(&r, Some(&path)).unwrap();

        // Append 3 garbage lines
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "garbage1").unwrap();
        writeln!(f, "garbage2").unwrap();
        writeln!(f, "garbage3").unwrap();

        record_run(&r, Some(&path)).unwrap();

        let result = load_records(Some(&path)).unwrap();
        assert_eq!(result.records.len(), 2);
        assert_eq!(result.skipped_lines, 3);
    }
}
