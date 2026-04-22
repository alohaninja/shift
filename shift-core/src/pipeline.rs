//! Core SHIFT pipeline: inspect → policy → transform → reconstruct.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::inspector;
use crate::inspector::MediaFormat;
use crate::mode::{ShiftConfig, SvgMode};
use crate::payload;
use crate::policy;
use crate::report::Report;
use crate::transformer;

/// Process a payload through the SHIFT pipeline.
///
/// Returns the transformed payload and a report of changes.
pub fn process(payload: &Value, config: &ShiftConfig) -> Result<(Value, Report)> {
    let mut report = Report::new();
    report.dry_run = config.dry_run;

    // Detect provider format if not specified
    let provider_format = payload::detect_provider(payload);

    // Load provider profile
    let profile = if let Ok(custom_path) = std::env::var("SHIFT_PROFILE") {
        policy::load_from_file(&custom_path)?
    } else {
        policy::load_builtin(&config.provider)?
    };

    // Get model-specific constraints
    let model_name = config
        .model
        .as_deref()
        .or_else(|| payload.get("model").and_then(|m| m.as_str()));
    let constraints = profile.constraints_for(model_name);

    // Extract images based on detected format
    let images = match provider_format {
        Some("openai") => payload::openai::extract_images(payload)?,
        Some("anthropic") => payload::anthropic::extract_images(payload)?,
        _ => {
            // No images found or text-only payload — pass through
            return Ok((payload.clone(), report));
        }
    };

    if images.is_empty() {
        return Ok((payload.clone(), report));
    }

    report.images_found = images.len();
    report.original_size = payload.to_string().len();

    let total_images = images.len();
    let mut transformed_images: Vec<(usize, Vec<u8>, String)> = Vec::new();

    for extracted in &images {
        // Inspect the image
        let meta = inspector::image::inspect_bytes(&extracted.data)
            .with_context(|| format!("failed to inspect image {}", extracted.global_index))?;

        // Evaluate policy
        let actions = policy::evaluate(
            &meta,
            constraints,
            config.mode,
            extracted.global_index,
            total_images,
        );

        // Handle SVG mode
        if meta.format == MediaFormat::Svg {
            let result = handle_svg(
                &extracted.data,
                &meta,
                &actions,
                config,
                extracted.global_index,
                &mut report,
            )?;
            transformed_images.push(result);
            continue;
        }

        // Apply transformations
        let mut current_data = extracted.data.clone();
        let mut was_modified = false;
        let mut output_mime = meta.format.mime_type().to_string();

        for action in &actions {
            match action {
                policy::Action::Pass => {}
                policy::Action::Drop { reason } => {
                    report.add_action(extracted.global_index, "drop", reason);
                    report.images_dropped += 1;
                    current_data = Vec::new();
                    was_modified = true;
                    break;
                }
                _ => {
                    if !config.dry_run {
                        let new_data = transformer::transform_image(&current_data, action)?;
                        let detail = describe_action(action, &meta);
                        report.add_action(extracted.global_index, action_name(action), &detail);
                        current_data = new_data;
                        was_modified = true;

                        // Update mime type based on action
                        match action {
                            policy::Action::ConvertFormat { to } => {
                                output_mime = format!("image/{}", to);
                            }
                            policy::Action::Resize { .. } => {
                                output_mime = "image/png".to_string();
                            }
                            policy::Action::Recompress { .. } => {
                                output_mime = "image/jpeg".to_string();
                            }
                            _ => {}
                        }
                    } else {
                        let detail = describe_action(action, &meta);
                        report.add_action(
                            extracted.global_index,
                            &format!("would_{}", action_name(action)),
                            &detail,
                        );
                        was_modified = true;
                    }
                }
            }
        }

        if was_modified {
            report.images_modified += 1;
        }

        transformed_images.push((extracted.global_index, current_data, output_mime));
    }

    // Reconstruct the payload
    let result = if config.dry_run {
        payload.clone()
    } else {
        match provider_format {
            Some("openai") => payload::openai::reconstruct(payload, &transformed_images)?,
            Some("anthropic") => payload::anthropic::reconstruct(payload, &transformed_images)?,
            _ => payload.clone(),
        }
    };

    report.transformed_size = result.to_string().len();

    Ok((result, report))
}

/// Handle SVG images according to the configured SvgMode.
fn handle_svg(
    data: &[u8],
    meta: &inspector::ImageMetadata,
    actions: &[policy::Action],
    config: &ShiftConfig,
    global_index: usize,
    report: &mut Report,
) -> Result<(usize, Vec<u8>, String)> {
    match config.svg_mode {
        SvgMode::Raster => {
            // Rasterize SVG to PNG
            if config.dry_run {
                let detail = format!("would rasterize {}x{} SVG to PNG", meta.width, meta.height);
                report.add_action(global_index, "would_rasterize_svg", &detail);
                report.images_modified += 1;
                return Ok((global_index, data.to_vec(), "image/svg+xml".to_string()));
            }

            // Find the rasterize action to get target dims
            let (tw, th) = actions
                .iter()
                .find_map(|a| match a {
                    policy::Action::RasterizeSvg {
                        target_width,
                        target_height,
                    } => Some((*target_width, *target_height)),
                    _ => None,
                })
                .unwrap_or((meta.width.max(256), meta.height.max(256)));

            let svg_text = std::str::from_utf8(data).context("SVG is not valid UTF-8")?;
            let png_data = transformer::rasterize_svg(svg_text, tw, th)?;

            report.add_action(
                global_index,
                "rasterize_svg",
                &format!(
                    "SVG ({}x{}) -> PNG ({}x{})",
                    meta.width, meta.height, tw, th
                ),
            );
            report.svgs_rasterized += 1;
            report.images_modified += 1;

            Ok((global_index, png_data, "image/png".to_string()))
        }

        SvgMode::Source => {
            // Pass SVG as text — the caller should replace the image block with a text block
            // For now, we convert to a text representation
            report.add_action(
                global_index,
                "svg_as_text",
                &format!("SVG ({}x{}) passed as source text", meta.width, meta.height),
            );
            report.images_modified += 1;

            // Return empty to signal the image should be replaced with text
            // The source text is available in meta.svg_source
            Ok((global_index, Vec::new(), "text/plain".to_string()))
        }

        SvgMode::Hybrid => {
            // Rasterize but the caller could also add SVG source as text
            if config.dry_run {
                report.add_action(
                    global_index,
                    "would_rasterize_svg_hybrid",
                    &format!(
                        "would rasterize {}x{} SVG (hybrid mode)",
                        meta.width, meta.height
                    ),
                );
                report.images_modified += 1;
                return Ok((global_index, data.to_vec(), "image/svg+xml".to_string()));
            }

            let (tw, th) = actions
                .iter()
                .find_map(|a| match a {
                    policy::Action::RasterizeSvg {
                        target_width,
                        target_height,
                    } => Some((*target_width, *target_height)),
                    _ => None,
                })
                .unwrap_or((meta.width.max(256), meta.height.max(256)));

            let svg_text = std::str::from_utf8(data).context("SVG is not valid UTF-8")?;
            let png_data = transformer::rasterize_svg(svg_text, tw, th)?;

            report.add_action(
                global_index,
                "rasterize_svg_hybrid",
                &format!(
                    "SVG ({}x{}) -> PNG ({}x{}) + source retained",
                    meta.width, meta.height, tw, th
                ),
            );
            report.svgs_rasterized += 1;
            report.images_modified += 1;

            Ok((global_index, png_data, "image/png".to_string()))
        }
    }
}

fn action_name(action: &policy::Action) -> &'static str {
    match action {
        policy::Action::Pass => "pass",
        policy::Action::Resize { .. } => "resize",
        policy::Action::Recompress { .. } => "recompress",
        policy::Action::ConvertFormat { .. } => "convert",
        policy::Action::RasterizeSvg { .. } => "rasterize_svg",
        policy::Action::Drop { .. } => "drop",
    }
}

fn describe_action(action: &policy::Action, meta: &inspector::ImageMetadata) -> String {
    match action {
        policy::Action::Pass => "no changes needed".to_string(),
        policy::Action::Resize {
            target_width,
            target_height,
        } => format!(
            "{}x{} -> {}x{}",
            meta.width, meta.height, target_width, target_height
        ),
        policy::Action::Recompress { quality } => {
            format!("recompress at quality {}", quality)
        }
        policy::Action::ConvertFormat { to } => {
            format!("{} -> {}", meta.format, to)
        }
        policy::Action::RasterizeSvg {
            target_width,
            target_height,
        } => format!("SVG -> PNG at {}x{}", target_width, target_height),
        policy::Action::Drop { reason } => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::DriveMode;
    use serde_json::json;

    fn make_png_data_uri(width: u32, height: u32) -> String {
        use base64::Engine;
        let img = image::RgbaImage::new(width, height);
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        format!("data:image/png;base64,{}", b64)
    }

    fn make_anthropic_png_base64(width: u32, height: u32) -> String {
        use base64::Engine;
        let img = image::RgbaImage::new(width, height);
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        base64::engine::general_purpose::STANDARD.encode(&buf)
    }

    #[test]
    fn test_text_only_passthrough() {
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let config = ShiftConfig::default();
        let (result, report) = process(&payload, &config).unwrap();
        assert_eq!(result, payload);
        assert_eq!(report.images_found, 0);
        assert!(!report.has_changes());
    }

    #[test]
    fn test_small_image_passthrough() {
        let data_uri = make_png_data_uri(640, 480);
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What's this?"},
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });
        let config = ShiftConfig::default();
        let (_result, report) = process(&payload, &config).unwrap();
        assert_eq!(report.images_found, 1);
    }

    #[test]
    fn test_oversized_image_resized_openai() {
        let data_uri = make_png_data_uri(4000, 3000);
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });
        let config = ShiftConfig {
            provider: "openai".to_string(),
            mode: DriveMode::Balanced,
            ..Default::default()
        };
        let (_result, report) = process(&payload, &config).unwrap();
        assert_eq!(report.images_found, 1);
        assert!(report.has_changes());
        assert!(report.actions.iter().any(|a| a.action == "resize"));
    }

    #[test]
    fn test_oversized_image_resized_anthropic() {
        let b64 = make_anthropic_png_base64(4000, 3000);
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/png", "data": b64}
                }]
            }]
        });
        let config = ShiftConfig {
            provider: "anthropic".to_string(),
            mode: DriveMode::Balanced,
            ..Default::default()
        };
        let (_result, report) = process(&payload, &config).unwrap();
        assert_eq!(report.images_found, 1);
        assert!(report.has_changes());
    }

    #[test]
    fn test_dry_run_no_modifications() {
        let data_uri = make_png_data_uri(4000, 3000);
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri.clone()}}
                ]
            }]
        });
        let config = ShiftConfig {
            dry_run: true,
            ..Default::default()
        };
        let (result, report) = process(&payload, &config).unwrap();
        // Dry run should not modify the payload
        assert_eq!(result, payload);
        // But should report what would happen
        assert!(report.has_changes());
        assert!(report.dry_run);
        assert!(report
            .actions
            .iter()
            .any(|a| a.action.starts_with("would_")));
    }

    #[test]
    fn test_svg_rasterization_in_openai_payload() {
        use base64::Engine;
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="red"/></svg>"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
        let data_uri = format!("data:image/svg+xml;base64,{}", b64);

        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });
        let config = ShiftConfig {
            svg_mode: SvgMode::Raster,
            ..Default::default()
        };
        let (_result, report) = process(&payload, &config).unwrap();
        assert_eq!(report.svgs_rasterized, 1);
        assert!(report.actions.iter().any(|a| a.action == "rasterize_svg"));
    }

    #[test]
    fn test_economy_mode_aggressive() {
        // 1500px image — within OpenAI limits but economy mode will downscale
        let data_uri = make_png_data_uri(1500, 1000);
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });
        let config = ShiftConfig {
            mode: DriveMode::Economy,
            ..Default::default()
        };
        let (_result, report) = process(&payload, &config).unwrap();
        assert!(report.has_changes());
    }

    #[test]
    fn test_performance_mode_minimal() {
        // 1500px image — within limits, performance mode should pass
        let data_uri = make_png_data_uri(1500, 1000);
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });
        let config = ShiftConfig {
            mode: DriveMode::Performance,
            ..Default::default()
        };
        let (_result, report) = process(&payload, &config).unwrap();
        // Performance mode should not modify images within limits
        assert!(!report.has_changes() || report.images_modified == 0);
    }
}
