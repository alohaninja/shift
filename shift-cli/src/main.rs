use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use shift_preflight::report::fmt_tokens;
use shift_preflight::{DriveMode, ImageMetrics, ShiftConfig, SvgMode, TokenSavings};
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

    /// Validate payload and preview image optimizations without transforming
    ///
    /// Runs the full inspection pipeline in dry-run mode and outputs a
    /// structured JSON report with environment checks, per-image analysis,
    /// token estimates, and optimization recommendations.
    Preflight {
        /// Input file (JSON request payload). Reads stdin if omitted.
        #[arg()]
        file: Option<String>,

        /// Target provider
        #[arg(short, long, default_value = "openai", value_parser = ["openai", "anthropic", "claude"])]
        provider: String,

        /// Drive mode
        #[arg(short, long, default_value = "balanced", value_parser = ["performance", "perf", "balanced", "bal", "economy", "eco"])]
        mode: String,

        /// Target model (overrides model in payload)
        #[arg(long)]
        model: Option<String>,

        /// Custom provider profile JSON file
        #[arg(long)]
        profile: Option<String>,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
}

/// Structured preflight report output (JSON to stdout).
#[derive(Debug, Serialize)]
struct PreflightReport {
    shift_version: String,
    provider: String,
    model_detected: Option<String>,
    mode: String,
    api_key_present: bool,
    api_key_env_var: String,
    images_found: usize,
    images_needing_transform: usize,
    images_ok: usize,
    total_original_bytes: usize,
    estimated_transformed_bytes: usize,
    estimated_byte_savings_pct: f64,
    token_estimate: TokenSavings,
    images: Vec<ImageMetrics>,
    warnings: Vec<String>,
    recommendations: Vec<String>,
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
            Commands::Preflight {
                file,
                provider,
                mode,
                model,
                profile,
                verbose,
            } => run_preflight(
                file,
                provider,
                mode,
                model.as_deref(),
                profile.as_deref(),
                *verbose,
            ),
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

fn run_preflight(
    file: &Option<String>,
    provider: &str,
    mode: &str,
    model: Option<&str>,
    profile: Option<&str>,
    verbose: bool,
) -> Result<()> {
    // 1. Read input
    let input = read_input(file)?;
    let payload: serde_json::Value =
        serde_json::from_str(&input).context("failed to parse input as JSON")?;

    // 2. Resolve provider
    let provider = if provider == "claude" {
        "anthropic"
    } else {
        provider
    };

    // 3. Detect model from payload if not overridden
    let model_detected = model.map(String::from).or_else(|| {
        payload
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from)
    });

    // 4. Check API key
    let api_key_env_var = match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        _ => "OPENAI_API_KEY",
    };
    let api_key_present = std::env::var(api_key_env_var).is_ok();

    // 5. Build config and run dry-run pipeline
    let drive_mode: DriveMode = mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let config = ShiftConfig {
        mode: drive_mode,
        svg_mode: SvgMode::Raster,
        provider: provider.to_string(),
        model: model.map(String::from),
        dry_run: true,
        verbose,
        profile_path: profile.map(String::from),
        limits: shift_preflight::SafetyLimits::default(),
    };

    let (_result, report) = shift_preflight::process(&payload, &config)?;

    // 6. Compute images needing transform vs ok
    let images_needing_transform = report
        .image_metrics
        .iter()
        .filter(|m| {
            m.original_width != m.transformed_width
                || m.original_height != m.transformed_height
                || m.format_before != m.format_after
        })
        .count();
    let images_ok = report.images_found.saturating_sub(images_needing_transform);

    // 7. Compute byte savings percentage
    let estimated_byte_savings_pct = if report.original_size > 0 {
        let diff = report.original_size as f64 - report.transformed_size as f64;
        (diff / report.original_size as f64) * 100.0
    } else {
        0.0
    };

    // 8. Build recommendations
    let mut recommendations = Vec::new();

    if !api_key_present {
        recommendations.push(format!(
            "Set {} environment variable before sending API requests",
            api_key_env_var
        ));
    }

    if images_needing_transform > 0 && drive_mode != DriveMode::Economy {
        // Estimate what economy mode would save
        let eco_config = ShiftConfig {
            mode: DriveMode::Economy,
            ..config.clone()
        };
        if let Ok((_eco_result, eco_report)) = shift_preflight::process(&payload, &eco_config) {
            let current_anthropic = report.token_savings.anthropic_after;
            let eco_anthropic = eco_report.token_savings.anthropic_after;
            if eco_anthropic < current_anthropic {
                let extra_pct = if current_anthropic > 0 {
                    ((current_anthropic - eco_anthropic) as f64 / current_anthropic as f64) * 100.0
                } else {
                    0.0
                };
                recommendations.push(format!(
                    "Economy mode would save an additional {:.0}% Anthropic tokens ({} -> {} tokens)",
                    extra_pct,
                    fmt_tokens(current_anthropic),
                    fmt_tokens(eco_anthropic),
                ));
            }
        }
    }

    if report.images_found == 0 {
        recommendations
            .push("No images found in payload — shift-ai optimization not needed".to_string());
    }

    if images_needing_transform == 0 && report.images_found > 0 {
        recommendations.push(
            "All images are within provider constraints — no optimization needed".to_string(),
        );
    }

    // 9. Build and output the preflight report
    let preflight = PreflightReport {
        shift_version: env!("CARGO_PKG_VERSION").to_string(),
        provider: provider.to_string(),
        model_detected,
        mode: format!("{}", drive_mode),
        api_key_present,
        api_key_env_var: api_key_env_var.to_string(),
        images_found: report.images_found,
        images_needing_transform,
        images_ok,
        total_original_bytes: report.original_size,
        estimated_transformed_bytes: report.transformed_size,
        estimated_byte_savings_pct,
        token_estimate: report.token_savings,
        images: report.image_metrics,
        warnings: report.warnings,
        recommendations,
    };

    let json = serde_json::to_string_pretty(&preflight)?;
    println!("{}", json);

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
