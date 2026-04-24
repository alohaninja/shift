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

    // Process (with timing)
    let start = std::time::Instant::now();
    let (result, report) = shift_preflight::process(&payload, &config)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    // Record stats (unless disabled or dry-run)
    if !cli.no_stats && !cli.dry_run && report.images_found > 0 {
        let record = shift_preflight::stats::record_from_report(&report, &provider, duration_ms);
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
    use colored::Colorize;

    let load_result = shift_preflight::stats::load_records(None)?;
    let records = load_result.records;
    let use_color = std::io::stdout().is_terminal();

    if records.is_empty() {
        println!("No SHIFT runs recorded yet. Stats are saved automatically after each run.");
        println!("Use --no-stats to disable.");
        return Ok(());
    }

    if daily {
        let days = shift_preflight::stats::daily_breakdown(&records);
        if format == Some("json") {
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
            if use_color {
                println!("{}", "SHIFT Daily Token Savings".bold().green());
                println!("{}", "═".repeat(58).green());
            } else {
                println!("=== SHIFT Daily Token Savings ===");
            }
            println!();
            println!(
                "{:<12} {:>5} {:>7} {:>15} {:>15}",
                "Date", "Runs", "Images", "OpenAI saved", "Anthropic saved"
            );
            println!("{}", "─".repeat(58));
            for d in &days {
                let oai = fmt_tokens(d.openai_saved);
                let ant = fmt_tokens(d.anthropic_saved);
                if use_color {
                    println!(
                        "{:<12} {:>5} {:>7} {:>15} {:>15}",
                        d.date,
                        d.runs,
                        d.images,
                        oai.green(),
                        ant.green(),
                    );
                } else {
                    println!(
                        "{:<12} {:>5} {:>7} {:>15} {:>15}",
                        d.date, d.runs, d.images, oai, ant,
                    );
                }
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
                "total_duration_ms": summary.total_duration_ms,
                "openai_tokens_before": summary.total_openai_before,
                "openai_tokens_after": summary.total_openai_after,
                "openai_tokens_saved": summary.openai_saved(),
                "openai_pct": summary.openai_pct(),
                "anthropic_tokens_before": summary.total_anthropic_before,
                "anthropic_tokens_after": summary.total_anthropic_after,
                "anthropic_tokens_saved": summary.anthropic_saved(),
                "anthropic_pct": summary.anthropic_pct(),
                "by_provider": summary.by_provider.iter().map(|p| {
                    serde_json::json!({
                        "provider": p.provider,
                        "runs": p.runs,
                        "images": p.images,
                        "tokens_saved": p.tokens_saved,
                        "overall_pct": p.overall_pct,
                        "avg_duration_ms": p.avg_duration_ms,
                    })
                }).collect::<Vec<_>>(),
                "by_action": summary.by_action.iter().map(|a| {
                    serde_json::json!({
                        "action": a.action,
                        "count": a.count,
                    })
                }).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        } else {
            // ── Header ──────────────────────────────────────────────
            if use_color {
                println!("{}", "SHIFT Token Savings".bold().green());
                println!("{}", "═".repeat(50).green());
            } else {
                println!("=== SHIFT Token Savings ===");
            }
            println!();

            // ── Summary stats ───────────────────────────────────────
            println!("{:<18}{}", "Total runs:", summary.total_runs);
            println!(
                "{:<18}{} ({} modified)",
                "Images processed:", summary.total_images, summary.total_modified
            );
            println!("{:<18}{}", "Bytes saved:", fmt_bytes(summary.bytes_saved()));
            if summary.total_duration_ms > 0 {
                let avg_ms = if summary.total_runs > 0 {
                    summary.total_duration_ms / summary.total_runs as u64
                } else {
                    0
                };
                println!(
                    "{:<18}{} (avg {})",
                    "Total exec time:",
                    fmt_duration(summary.total_duration_ms),
                    fmt_duration(avg_ms)
                );
            }
            if load_result.skipped_lines > 0 {
                let msg = format!(
                    "{} corrupted stats line(s) skipped",
                    load_result.skipped_lines
                );
                if use_color {
                    println!("{:<18}{}", "Warning:", msg.yellow());
                } else {
                    println!("{:<18}{}", "Warning:", msg);
                }
            }
            println!();

            // ── Token savings ────────────────────────────────────────
            if summary.total_openai_before > 0 {
                let pct_str = format!("{:.1}%", summary.openai_pct());
                if use_color {
                    println!(
                        "{:<18}{} -> {} tokens  ({})",
                        "OpenAI tokens:",
                        fmt_tokens(summary.total_openai_before),
                        fmt_tokens(summary.total_openai_after),
                        colorize_pct(&pct_str, summary.openai_pct(), use_color),
                    );
                } else {
                    println!(
                        "{:<18}{} -> {} tokens  ({} saved)",
                        "OpenAI tokens:",
                        fmt_tokens(summary.total_openai_before),
                        fmt_tokens(summary.total_openai_after),
                        pct_str,
                    );
                }
            }
            if summary.total_anthropic_before > 0 {
                let pct_str = format!("{:.1}%", summary.anthropic_pct());
                if use_color {
                    println!(
                        "{:<18}{} -> {} tokens  ({})",
                        "Anthropic tokens:",
                        fmt_tokens(summary.total_anthropic_before),
                        fmt_tokens(summary.total_anthropic_after),
                        colorize_pct(&pct_str, summary.anthropic_pct(), use_color),
                    );
                } else {
                    println!(
                        "{:<18}{} -> {} tokens  ({} saved)",
                        "Anthropic tokens:",
                        fmt_tokens(summary.total_anthropic_before),
                        fmt_tokens(summary.total_anthropic_after),
                        pct_str,
                    );
                }
            }

            // ── Efficiency meter ─────────────────────────────────────
            // Use the best savings percentage across providers
            let best_pct = if summary.anthropic_pct() > summary.openai_pct() {
                summary.anthropic_pct()
            } else {
                summary.openai_pct()
            };
            println!();
            print_efficiency_meter(best_pct, use_color);

            // ── By Provider ──────────────────────────────────────────
            if summary.by_provider.len() > 1 {
                println!();
                if use_color {
                    println!("{}", "By Provider".bold().green());
                } else {
                    println!("By Provider");
                }
                println!();
                println!(
                    " {:<3}{:<14}{:>6}{:>9}{:>14}{:>8}{:>8}  Impact",
                    "#", "Provider", "Runs", "Images", "Tokens Saved", "Avg%", "Time"
                );
                println!(" {}", "─".repeat(70));
                let max_saved = summary
                    .by_provider
                    .iter()
                    .map(|p| p.tokens_saved)
                    .max()
                    .unwrap_or(1);
                for (i, p) in summary.by_provider.iter().enumerate() {
                    let pct_str = format!("{:.1}%", p.overall_pct);
                    let bar = mini_bar(p.tokens_saved, max_saved, 10, use_color);
                    if use_color {
                        println!(
                            " {:<3}{:<14}{:>6}{:>9}{:>14}{:>8}{:>8}  {}",
                            format!("{}.", i + 1),
                            p.provider,
                            p.runs,
                            p.images,
                            fmt_tokens(p.tokens_saved),
                            colorize_pct(&pct_str, p.overall_pct, use_color),
                            fmt_duration(p.avg_duration_ms),
                            bar,
                        );
                    } else {
                        println!(
                            " {:<3}{:<14}{:>6}{:>9}{:>14}{:>8}{:>8}  {}",
                            format!("{}.", i + 1),
                            p.provider,
                            p.runs,
                            p.images,
                            fmt_tokens(p.tokens_saved),
                            pct_str,
                            fmt_duration(p.avg_duration_ms),
                            bar,
                        );
                    }
                }
            }

            // ── By Action ────────────────────────────────────────────
            if !summary.by_action.is_empty() {
                println!();
                if use_color {
                    println!("{}", "By Action".bold().green());
                } else {
                    println!("By Action");
                }
                println!();
                println!(" {:<3}{:<20}{:>8}", "#", "Action", "Count");
                println!(" {}", "─".repeat(32));
                for (i, a) in summary.by_action.iter().enumerate() {
                    println!(
                        " {:<3}{:<20}{:>8}",
                        format!("{}.", i + 1),
                        a.action,
                        a.count,
                    );
                }
            }

            // ── Daily sparkline (last 30 days) ───────────────────────
            let days = shift_preflight::stats::daily_breakdown(&records);
            if days.len() > 1 {
                println!();
                if use_color {
                    println!("{}", "Last 30 Days".bold().green());
                } else {
                    println!("Last 30 Days");
                }
                println!();
                print_sparkline(&days, use_color);
            }
        }
    }

    Ok(())
}

/// Print an RTK-style efficiency meter bar.
fn print_efficiency_meter(pct: f64, use_color: bool) {
    use colored::Colorize;
    let width = 24usize;
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let meter = format!("{}{}", "█".repeat(filled), "░".repeat(width - filled));
    let pct_str = format!("{:.1}%", pct);
    if use_color {
        println!(
            "{:<18}{} {}",
            "Efficiency meter:",
            meter.green(),
            colorize_pct(&pct_str, pct, use_color),
        );
    } else {
        println!("{:<18}{} {}", "Efficiency meter:", meter, pct_str,);
    }
}

/// Print an ASCII sparkline graph of daily token savings (last 30 days).
fn print_sparkline(days: &[shift_preflight::stats::DailyGain], use_color: bool) {
    use colored::Colorize;

    // Take last 30 days
    let days: Vec<_> = if days.len() > 30 {
        days[days.len() - 30..].to_vec()
    } else {
        days.to_vec()
    };

    // Use combined tokens saved (best of openai/anthropic per day)
    let values: Vec<(String, u64)> = days
        .iter()
        .map(|d| {
            let saved = d.openai_saved.max(d.anthropic_saved);
            // Shorten date: "2026-04-23" -> "04-23"
            let short = if d.date.len() >= 10 {
                d.date[5..10].to_string()
            } else {
                d.date.clone()
            };
            (short, saved)
        })
        .collect();

    let max_val = values.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1);
    let bar_width = 36usize;

    for (date, value) in &values {
        let filled = ((*value as f64 / max_val as f64) * bar_width as f64).round() as usize;
        let filled = filled.min(bar_width);
        let bar = "█".repeat(filled);
        let tokens = fmt_short_tokens(*value);
        if use_color {
            println!(
                " {} │{:<width$} {}",
                date,
                bar.cyan(),
                tokens,
                width = bar_width
            );
        } else {
            println!(" {} │{:<width$} {}", date, bar, tokens, width = bar_width);
        }
    }
}

/// Format tokens in compact form: 1.2K, 3.5M, etc.
fn fmt_short_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// Color a percentage string by tier: green >= 50%, yellow >= 20%, red < 20%.
fn colorize_pct(s: &str, pct: f64, use_color: bool) -> String {
    use colored::Colorize;
    if !use_color {
        return s.to_string();
    }
    if pct >= 50.0 {
        s.green().bold().to_string()
    } else if pct >= 20.0 {
        s.yellow().bold().to_string()
    } else {
        s.red().bold().to_string()
    }
}

/// Build a mini impact bar (RTK-style).
fn mini_bar(value: u64, max: u64, width: usize, use_color: bool) -> String {
    use colored::Colorize;
    if max == 0 {
        return "░".repeat(width);
    }
    let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(width - filled));
    if use_color {
        bar.cyan().to_string()
    } else {
        bar
    }
}

/// Format milliseconds as human-readable duration.
fn fmt_duration(ms: u64) -> String {
    if ms == 0 {
        return "0ms".to_string();
    }
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1_000;
        format!("{}m{:02}s", mins, secs)
    }
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
