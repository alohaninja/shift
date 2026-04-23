//! Integration tests for the SHIFT pipeline using fixture files.
//!
//! These tests exercise the full pipeline end-to-end with real payloads
//! loaded from tests/fixtures/.

use shift_preflight::{pipeline, DriveMode, Report, ShiftConfig, SvgMode};

fn load_fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR")
            .replace("/shift-core", "")
            .replace("/shift-cli", ""),
        name
    );
    // Try the path as-is first, then try from workspace root
    std::fs::read_to_string(&path)
        .or_else(|_| {
            let alt = format!("tests/fixtures/{}", name);
            std::fs::read_to_string(&alt)
        })
        .unwrap_or_else(|e| panic!("failed to load fixture '{}': {} (tried {})", name, e, path))
}

fn process_fixture(fixture: &str, provider: &str, mode: DriveMode) -> (serde_json::Value, Report) {
    let json_str = load_fixture(fixture);
    let payload: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let config = ShiftConfig {
        provider: provider.to_string(),
        mode,
        ..Default::default()
    };
    pipeline::process(&payload, &config).unwrap()
}

// ---- OpenAI fixture tests ----

#[test]
fn test_openai_fixture_balanced_resizes() {
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);
    assert_eq!(report.images_found, 1);
    assert!(report.has_changes(), "oversized image should be resized");
    assert!(
        report.actions.iter().any(|a| a.action == "resize"),
        "expected a resize action"
    );
}

#[test]
fn test_openai_fixture_performance_resizes() {
    // 4000x3000 exceeds OpenAI's 2048 limit even in performance mode
    let (_result, report) =
        process_fixture("openai_request.json", "openai", DriveMode::Performance);
    assert!(report.has_changes());
}

#[test]
fn test_openai_fixture_economy_aggressive() {
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Economy);
    assert!(report.has_changes());
    // Economy should resize more aggressively
    if let Some(resize_action) = report.actions.iter().find(|a| a.action == "resize") {
        // The detail contains target dimensions like "4000x3000 -> 1024x768"
        assert!(
            resize_action.detail.contains("1024") || resize_action.detail.contains("768"),
            "economy mode should target 1024px max, got: {}",
            resize_action.detail
        );
    }
}

#[test]
fn test_openai_fixture_produces_valid_json() {
    let (result, _report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);
    // Result should be valid JSON with the expected structure
    assert!(result.get("model").is_some());
    assert!(result.get("messages").is_some());
    let messages = result["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
}

#[test]
fn test_openai_fixture_dry_run() {
    let json_str = load_fixture("openai_request.json");
    let payload: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Balanced,
        dry_run: true,
        ..Default::default()
    };
    let (result, report) = pipeline::process(&payload, &config).unwrap();
    // Dry run: payload should be unchanged
    assert_eq!(result, payload);
    assert!(report.dry_run);
    assert!(report.has_changes());
}

// ---- Anthropic fixture tests ----

#[test]
fn test_anthropic_fixture_balanced_resizes() {
    let (_result, report) =
        process_fixture("anthropic_request.json", "anthropic", DriveMode::Balanced);
    assert_eq!(report.images_found, 1);
    // 4000x3000 = 12 MP, exceeds Anthropic's 1.15 MP limit
    assert!(report.has_changes());
}

#[test]
fn test_anthropic_fixture_produces_valid_structure() {
    let (result, _report) =
        process_fixture("anthropic_request.json", "anthropic", DriveMode::Balanced);
    assert!(result.get("model").is_some());
    assert!(result.get("messages").is_some());
    // Anthropic images should have source.type = "base64" after transformation
    let content = result["messages"][0]["content"].as_array().unwrap();
    for part in content {
        if part.get("type").and_then(|t| t.as_str()) == Some("image") {
            let source = &part["source"];
            assert_eq!(source["type"], "base64");
            assert!(source.get("data").is_some());
            assert!(source.get("media_type").is_some());
        }
    }
}

// ---- SVG fixture tests ----

#[test]
fn test_svg_fixture_rasterize() {
    // Build a payload with the SVG fixture inline
    let svg_content = load_fixture("test.svg");
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        svg_content.as_bytes(),
    );
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "image_url",
                "image_url": {"url": format!("data:image/svg+xml;base64,{}", b64)}
            }]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        svg_mode: SvgMode::Raster,
        ..Default::default()
    };
    let (result, report) = pipeline::process(&payload, &config).unwrap();

    assert_eq!(report.svgs_rasterized, 1);
    assert!(report.has_changes());

    // The output should have PNG, not SVG
    let url = result["messages"][0]["content"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    assert!(
        url.starts_with("data:image/png;base64,"),
        "expected PNG data URI after SVG rasterization"
    );
}

use base64::Engine as _;

#[test]
fn test_svg_fixture_source_mode() {
    let svg_content = load_fixture("test.svg");
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg_content.as_bytes());
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "image_url",
                "image_url": {"url": format!("data:image/svg+xml;base64,{}", b64)}
            }]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        svg_mode: SvgMode::Source,
        ..Default::default()
    };
    let (_result, report) = pipeline::process(&payload, &config).unwrap();

    assert!(report.has_changes());
    assert!(
        report
            .actions
            .iter()
            .any(|a| a.action == "svg_dropped_as_source"),
        "source mode should produce svg_dropped_as_source action"
    );
    assert!(
        report.images_dropped > 0,
        "source mode should count as dropped"
    );
}

// ---- Cross-mode comparison ----

#[test]
fn test_mode_comparison_openai() {
    let json_str = load_fixture("openai_request.json");
    let payload: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let mut reports = Vec::new();
    for mode in [
        DriveMode::Performance,
        DriveMode::Balanced,
        DriveMode::Economy,
    ] {
        let config = ShiftConfig {
            provider: "openai".to_string(),
            mode,
            ..Default::default()
        };
        let (_result, report) = pipeline::process(&payload, &config).unwrap();
        reports.push((mode, report));
    }

    // All modes should process the image (it's oversized)
    for (mode, report) in &reports {
        assert!(
            report.has_changes(),
            "{} mode should modify oversized image",
            mode
        );
    }
}

// ---- Real-world image tests (shift-icon.png: 1254x1254, ~1.2MB) ----

fn load_fixture_bytes(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR")
            .replace("/shift-core", "")
            .replace("/shift-cli", ""),
        name
    );
    std::fs::read(&path)
        .or_else(|_| std::fs::read(format!("tests/fixtures/{}", name)))
        .unwrap_or_else(|e| panic!("failed to load fixture '{}': {}", name, e))
}

#[test]
fn test_real_image_openai_balanced() {
    let icon_bytes = load_fixture_bytes("shift-icon.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&icon_bytes);
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What does this app icon depict?"},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Balanced,
        ..Default::default()
    };
    let (result, report) = pipeline::process(&payload, &config).unwrap();

    // 1254x1254 is within OpenAI's 2048 limit, so balanced should pass it through
    assert_eq!(report.images_found, 1);
    // Verify output is valid
    assert!(result.get("messages").is_some());
}

#[test]
fn test_real_image_openai_economy_downscales() {
    let icon_bytes = load_fixture_bytes("shift-icon.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&icon_bytes);
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Economy,
        ..Default::default()
    };
    let (_result, report) = pipeline::process(&payload, &config).unwrap();

    // 1254px > 1024 economy cap, so economy mode should downscale
    assert!(
        report.has_changes(),
        "economy mode should downscale 1254px image to 1024"
    );
    assert!(
        report.actions.iter().any(|a| a.action == "resize"),
        "expected resize action for 1254px image in economy mode"
    );
}

#[test]
fn test_real_image_anthropic_megapixel_check() {
    let icon_bytes = load_fixture_bytes("shift-icon.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&icon_bytes);
    let payload = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/png", "data": b64}
                },
                {"type": "text", "text": "Describe this icon"}
            ]
        }]
    });

    let config = ShiftConfig {
        provider: "anthropic".to_string(),
        mode: DriveMode::Balanced,
        ..Default::default()
    };
    let (result, report) = pipeline::process(&payload, &config).unwrap();

    // 1254x1254 = 1.57 MP, exceeds Anthropic's 1.15 MP limit
    assert_eq!(report.images_found, 1);
    assert!(
        report.has_changes(),
        "1254x1254 (1.57MP) should exceed Anthropic 1.15MP limit"
    );
    assert!(
        report.actions.iter().any(|a| a.action == "resize"),
        "expected resize for megapixel constraint"
    );

    // Verify the output image is valid base64 PNG
    let content = result["messages"][0]["content"].as_array().unwrap();
    let image_part = content
        .iter()
        .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("image"))
        .expect("should still have an image block");
    let data = image_part["source"]["data"].as_str().unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert!(
        decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
        "output should be valid PNG"
    );
}

#[test]
fn test_real_image_performance_passthrough() {
    let icon_bytes = load_fixture_bytes("shift-icon.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&icon_bytes);
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Performance,
        ..Default::default()
    };
    let (_result, report) = pipeline::process(&payload, &config).unwrap();

    // 1254px is under OpenAI's 2048 limit, performance mode should pass through
    assert_eq!(report.images_found, 1);
    assert!(
        !report.has_changes(),
        "1254px is within OpenAI limits, performance mode should not modify"
    );
}

#[test]
fn test_real_image_size_reduction_economy() {
    let icon_bytes = load_fixture_bytes("shift-icon.png");
    let original_size = icon_bytes.len();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&icon_bytes);
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }]
    });

    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Economy,
        ..Default::default()
    };
    let (result, report) = pipeline::process(&payload, &config).unwrap();

    assert!(report.has_changes());

    // Extract the output image and verify it's smaller
    let url = result["messages"][0]["content"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    let b64_part = url.split(',').nth(1).unwrap();
    let output_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_part)
        .unwrap();

    // Downscaled image should be significantly smaller than 1.2MB original
    assert!(
        output_bytes.len() < original_size,
        "economy mode output ({} bytes) should be smaller than original ({} bytes)",
        output_bytes.len(),
        original_size
    );
}

// ── Token savings integration tests ─────────────────────────────────

#[test]
fn test_image_metrics_populated_after_processing() {
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);

    // image_metrics should have one entry per image found
    assert_eq!(report.image_metrics.len(), report.images_found);
    assert!(!report.image_metrics.is_empty());

    let m = &report.image_metrics[0];
    // Original dims should be non-zero
    assert!(m.original_width > 0);
    assert!(m.original_height > 0);
    // Token estimates should be non-zero
    assert!(m.tokens_before.openai_tokens > 0);
    assert!(m.tokens_before.anthropic_tokens > 0);
    assert!(m.tokens_after.openai_tokens > 0);
    assert!(m.tokens_after.anthropic_tokens > 0);
}

#[test]
fn test_token_savings_aggregated() {
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);

    let ts = &report.token_savings;
    // Aggregate should match sum of per-image metrics
    let sum_openai_before: u64 = report
        .image_metrics
        .iter()
        .map(|m| m.tokens_before.openai_tokens)
        .sum();
    let sum_openai_after: u64 = report
        .image_metrics
        .iter()
        .map(|m| m.tokens_after.openai_tokens)
        .sum();
    assert_eq!(ts.openai_before, sum_openai_before);
    assert_eq!(ts.openai_after, sum_openai_after);
}

#[test]
fn test_image_metrics_tracks_resize() {
    // The fixture has a 4000x3000 image that gets resized
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);

    let m = &report.image_metrics[0];
    // Original should be larger than transformed
    assert!(
        m.original_width > m.transformed_width || m.original_height > m.transformed_height,
        "expected resize: {}x{} -> {}x{}",
        m.original_width,
        m.original_height,
        m.transformed_width,
        m.transformed_height
    );
}

#[test]
fn test_image_metrics_anthropic_provider() {
    let (_result, report) =
        process_fixture("anthropic_request.json", "anthropic", DriveMode::Balanced);

    assert!(!report.image_metrics.is_empty());
    let m = &report.image_metrics[0];
    // Anthropic tokens should be populated
    assert!(m.tokens_before.anthropic_tokens > 0);
    assert!(m.tokens_after.anthropic_tokens > 0);
}

#[test]
fn test_image_metrics_dry_run() {
    let json_str = load_fixture("openai_request.json");
    let payload: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let config = ShiftConfig {
        provider: "openai".to_string(),
        mode: DriveMode::Balanced,
        dry_run: true,
        ..Default::default()
    };
    let (_result, report) = pipeline::process(&payload, &config).unwrap();

    // Even in dry-run, metrics should be populated with estimated dims
    assert!(!report.image_metrics.is_empty());
    let m = &report.image_metrics[0];
    assert!(m.original_width > 0);
    assert!(m.tokens_before.openai_tokens > 0);
}

#[test]
fn test_report_serializes_to_json() {
    let (_result, report) = process_fixture("openai_request.json", "openai", DriveMode::Balanced);

    // Report should serialize to valid JSON (for -o json-report)
    let json = serde_json::to_string_pretty(&report).expect("report should serialize to JSON");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("serialized report should parse back");

    // Verify key fields exist
    assert!(parsed["image_metrics"].is_array());
    assert!(parsed["token_savings"].is_object());
    assert!(parsed["token_savings"]["openai_before"].is_u64());
    assert!(parsed["token_savings"]["anthropic_before"].is_u64());

    // image_metrics should have nested token estimates
    if let Some(first) = parsed["image_metrics"].as_array().and_then(|a| a.first()) {
        assert!(first["tokens_before"]["openai_tokens"].is_u64());
        assert!(first["tokens_before"]["anthropic_tokens"].is_u64());
        assert!(first["original_width"].is_u64());
        assert!(first["transformed_width"].is_u64());
    }
}
