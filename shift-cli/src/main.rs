use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use shift_preflight::report::fmt_tokens;
use shift_preflight::{DriveMode, ShiftConfig, SvgMode};
use std::io::{IsTerminal, Read};

/// SHIFT — Smart Hybrid Input Filtering & Transformation
///
/// A multimodal preflight layer that automatically adapts inputs
/// before they are sent to an AI model.
///
/// Reads a JSON request payload from stdin or a file, transforms
/// images to meet provider constraints, and writes the safe payload
/// to stdout.
#[derive(Parser, Debug)]
#[command(name = "shift-ai", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Input file (JSON request payload). Reads stdin if omitted.
    #[arg()]
    file: Option<String>,

    /// Target provider
    #[arg(short, long, default_value = "openai", value_parser = ["openai", "anthropic", "claude"])]
    provider: String,

    /// Drive mode
    #[arg(short, long, default_value = "balanced", value_parser = ["performance", "perf", "balanced", "bal", "economy", "eco"])]
    mode: String,

    /// SVG handling mode
    #[arg(long, default_value = "raster", value_parser = ["raster", "source", "hybrid"])]
    svg_mode: String,

    /// Output format
    ///   json        = transformed payload (default)
    ///   report      = human-readable transformation report
    ///   json-report = machine-readable report as JSON (for dashboards)
    ///   both        = report to stderr + payload to stdout
    #[arg(short, long, default_value = "json", value_parser = ["json", "report", "json-report", "both"])]
    output: String,

    /// Show what would change without modifying
    #[arg(long)]
    dry_run: bool,

    /// Custom provider profile JSON file
    #[arg(long)]
    profile: Option<String>,

    /// Target model (overrides model in payload)
    #[arg(long)]
    model: Option<String>,

    /// Disable saving run statistics to ~/.shift/stats.jsonl
    #[arg(long)]
    no_stats: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show cumulative token savings across all recorded runs
    Gain {
        /// Show day-by-day breakdown
        #[arg(long)]
        daily: bool,

        /// Output as JSON
        #[arg(long, value_parser = ["json"])]
        format: Option<String>,
    },
}

/// Maximum stdin input size: 500 MB
const MAX_STDIN_BYTES: u64 = 500_000_000;

fn main() {
    if let Err(e) = run() {
        eprintln!("shift-ai: error: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Handle subcommands
    if let Some(cmd) = &cli.command {
        return match cmd {
            Commands::Gain { daily, format } => run_gain(*daily, format.as_deref()),
        };
    }

    // Read input
    let input = read_input(&cli.file)?;

    // Parse JSON
    let payload: serde_json::Value =
        serde_json::from_str(&input).context("failed to parse input as JSON")?;

    // Build config
    let drive_mode: DriveMode = cli.mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let svg_mode: SvgMode = cli
        .svg_mode
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let provider = if cli.provider == "claude" {
        "anthropic".to_string()
    } else {
        cli.provider.clone()
    };

    // Fix #7: Thread profile_path through ShiftConfig instead of env var
    let config = ShiftConfig {
        mode: drive_mode,
        svg_mode,
        provider: provider.clone(),
        model: cli.model,
        dry_run: cli.dry_run,
        verbose: cli.verbose,
        profile_path: cli.profile,
        limits: shift_preflight::SafetyLimits::default(),
    };

    if cli.verbose {
        eprintln!(
            "shift-ai: mode={}, provider={}, svg_mode={}, dry_run={}",
            config.mode, config.provider, config.svg_mode, config.dry_run
        );
    }

    // Process
    let (result, report) = shift_preflight::process(&payload, &config)?;

    // Record stats (unless disabled or dry-run)
    if !cli.no_stats && !cli.dry_run && report.images_found > 0 {
        let record = shift_preflight::stats::record_from_report(&report, &provider);
        if let Err(e) = shift_preflight::stats::record_run(&record, None) {
            eprintln!("shift-ai: warning: failed to save stats: {}", e);
        }
    }

    // Output
    match cli.output.as_str() {
        "json" => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);

            if cli.verbose || cli.dry_run {
                eprintln!("{}", report);
            }
        }
        "report" => {
            println!("{}", report);
        }
        "json-report" => {
            let json = serde_json::to_string_pretty(&report)?;
            println!("{}", json);
        }
        "both" => {
            eprintln!("{}", report);
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn run_gain(daily: bool, format: Option<&str>) -> Result<()> {
    let load_result = shift_preflight::stats::load_records(None)?;
    let records = load_result.records;

    if records.is_empty() {
        println!("No SHIFT runs recorded yet. Stats are saved automatically after each run.");
        println!("Use --no-stats to disable.");
        return Ok(());
    }

    if daily {
        let days = shift_preflight::stats::daily_breakdown(&records);
        if format == Some("json") {
            // Serialize daily data as JSON
            let json_days: Vec<serde_json::Value> = days
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "date": d.date,
                        "runs": d.runs,
                        "images": d.images,
                        "openai_tokens_saved": d.openai_saved,
                        "anthropic_tokens_saved": d.anthropic_saved,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json_days)?);
        } else {
            println!("=== SHIFT Daily Token Savings ===\n");
            println!(
                "{:<12} {:>5} {:>7} {:>15} {:>15}",
                "Date", "Runs", "Images", "OpenAI saved", "Anthropic saved"
            );
            println!("{}", "-".repeat(58));
            for d in &days {
                println!(
                    "{:<12} {:>5} {:>7} {:>15} {:>15}",
                    d.date,
                    d.runs,
                    d.images,
                    fmt_tokens(d.openai_saved),
                    fmt_tokens(d.anthropic_saved),
                );
            }
        }
    } else {
        let summary = shift_preflight::stats::summarize(&records);
        if format == Some("json") {
            let json = serde_json::json!({
                "total_runs": summary.total_runs,
                "total_images": summary.total_images,
                "total_modified": summary.total_modified,
                "bytes_saved": summary.bytes_saved(),
                "openai_tokens_before": summary.total_openai_before,
                "openai_tokens_after": summary.total_openai_after,
                "openai_tokens_saved": summary.openai_saved(),
                "openai_pct": summary.openai_pct(),
                "anthropic_tokens_before": summary.total_anthropic_before,
                "anthropic_tokens_after": summary.total_anthropic_after,
                "anthropic_tokens_saved": summary.anthropic_saved(),
                "anthropic_pct": summary.anthropic_pct(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        } else {
            println!("=== SHIFT Cumulative Savings ===\n");
            println!("Runs:     {}", summary.total_runs);
            println!(
                "Images:   {} processed, {} modified",
                summary.total_images, summary.total_modified
            );
            println!("Bytes:    {} saved", fmt_bytes(summary.bytes_saved()));
            if load_result.skipped_lines > 0 {
                println!(
                    "Warning:  {} corrupted stats line(s) skipped",
                    load_result.skipped_lines
                );
            }
            println!();
            println!("Token Savings (estimated):");
            if summary.total_openai_before > 0 {
                println!(
                    "  OpenAI:    {} -> {} tokens  ({:.1}% saved)",
                    fmt_tokens(summary.total_openai_before),
                    fmt_tokens(summary.total_openai_after),
                    summary.openai_pct()
                );
            }
            if summary.total_anthropic_before > 0 {
                println!(
                    "  Anthropic: {} -> {} tokens  ({:.1}% saved)",
                    fmt_tokens(summary.total_anthropic_before),
                    fmt_tokens(summary.total_anthropic_after),
                    summary.anthropic_pct()
                );
            }
        }
    }

    Ok(())
}

fn fmt_bytes(n: u64) -> String {
    if n < 1_024 {
        format!("{} B", n)
    } else if n < 1_048_576 {
        format!("{:.1} KB", n as f64 / 1_024.0)
    } else {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    }
}

fn read_input(file: &Option<String>) -> Result<String> {
    match file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path))
        }
        None => {
            // Fix #18: Use std::io::IsTerminal instead of unmaintained `atty`
            if std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "no input provided. Usage:\n  shift-ai <file.json>\n  cat request.json | shift-ai\n  shift-ai gain       (show cumulative savings)"
                );
            }
            // Fix #19: Limit stdin read size
            let mut buf = String::new();
            std::io::stdin()
                .take(MAX_STDIN_BYTES)
                .read_to_string(&mut buf)
                .context("failed to read stdin")?;
            Ok(buf)
        }
    }
}
