use anyhow::{Context, Result};
use clap::Parser;
use shift_core::{DriveMode, ShiftConfig, SvgMode};
use std::io::Read;

/// SHIFT — Smart Hybrid Input Filtering & Transformation
///
/// A multimodal preflight layer that automatically adapts inputs
/// before they are sent to an AI model.
///
/// Reads a JSON request payload from stdin or a file, transforms
/// images to meet provider constraints, and writes the safe payload
/// to stdout.
#[derive(Parser, Debug)]
#[command(name = "shift", version, about)]
struct Cli {
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

    /// Output format (json = transformed payload, report = transformation report)
    #[arg(short, long, default_value = "json", value_parser = ["json", "report", "both"])]
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

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("shift: error: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

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

    let config = ShiftConfig {
        mode: drive_mode,
        svg_mode,
        provider,
        model: cli.model,
        dry_run: cli.dry_run,
        verbose: cli.verbose,
    };

    // Set custom profile path via env if provided
    if let Some(ref profile_path) = cli.profile {
        std::env::set_var("SHIFT_PROFILE", profile_path);
    }

    if cli.verbose {
        eprintln!(
            "shift: mode={}, provider={}, svg_mode={}, dry_run={}",
            config.mode, config.provider, config.svg_mode, config.dry_run
        );
    }

    // Process
    let (result, report) = shift_core::process(&payload, &config)?;

    // Output
    match cli.output.as_str() {
        "json" => {
            // Payload to stdout, report to stderr if verbose
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);

            if cli.verbose || cli.dry_run {
                eprintln!("{}", report);
            }
        }
        "report" => {
            // Report only (to stdout)
            println!("{}", report);
        }
        "both" => {
            // Report to stderr, payload to stdout
            eprintln!("{}", report);
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn read_input(file: &Option<String>) -> Result<String> {
    match file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path))
        }
        None => {
            // Check if stdin has data
            if atty::is(atty::Stream::Stdin) {
                anyhow::bail!(
                    "no input provided. Usage:\n  shift <file.json>\n  cat request.json | shift"
                );
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read stdin")?;
            Ok(buf)
        }
    }
}
