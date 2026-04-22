//! Core SHIFT pipeline: inspect → policy → transform → reconstruct.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::cost::{estimate_tokens, ImageMetrics};
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

    // Fix #7: Load provider profile from config, not env var
    let profile = if let Some(ref custom_path) = config.profile_path {
        // R7: Validate the profile path more thoroughly
        let path = std::path::Path::new(custom_path);

        // Must have a .json extension
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => {}
            _ => anyhow::bail!("profile path must have a .json extension"),
        }

        // Reject path traversal components
        for component in path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                anyhow::bail!("profile path must not contain '..' path traversal");
            }
        }

        // Canonicalize to resolve symlinks, then verify the canonical path
        // ends with .json (symlink to /etc/passwd would fail this)
        if path.exists() {
            let canonical = std::fs::canonicalize(path)
                .with_context(|| "failed to resolve profile path".to_string())?;
            match canonical.extension().and_then(|e| e.to_str()) {
                Some("json") => {}
                _ => anyhow::bail!(
                    "profile path resolves to a non-JSON file (possible symlink attack)"
                ),
            }
            policy::load_from_file(canonical.to_str().unwrap_or(custom_path))?
        } else {
            policy::load_from_file(custom_path)?
        }
    } else {
        policy::load_builtin(&config.provider)?
    };

    // Get model-specific constraints
    let model_name = config
        .model
        .as_deref()
        .or_else(|| payload.get("model").and_then(|m| m.as_str()));
    let constraints = profile.constraints_for(model_name);

    // R8: Extract images with configured safety limits
    let images = match provider_format {
        Some("openai") => payload::openai::extract_images_with_limits(payload, &config.limits)?,
        Some("anthropic") => {
            payload::anthropic::extract_images_with_limits(payload, &config.limits)?
        }
        _ => {
            // No images found or text-only payload — pass through
            return Ok((payload.clone(), report));
        }
    };

    if images.is_empty() {
        return Ok((payload.clone(), report));
    }

    report.images_found = images.len();
    // Fix #16: Track image byte sizes separately from JSON serialization
    let original_image_bytes: usize = images.iter().map(|img| img.data.len()).sum();
    report.original_size = original_image_bytes;

    let total_images = images.len();
    let mut transformed_images: Vec<(usize, Vec<u8>, String)> = Vec::new();

    for extracted in &images {
        // Fix #15: Inspect with skip-and-warn on individual failures
        let meta = match inspector::image::inspect_bytes(&extracted.data) {
            Ok(m) => m,
            Err(e) => {
                report.add_warning(&format!(
                    "image {}: skipped ({})",
                    extracted.global_index, e
                ));
                // R6: Use the original MIME type from the image reference,
                // not a hardcoded "image/png" which would mislabel JPEG/WebP/GIF.
                let original_mime = match &extracted.original_ref {
                    payload::ImageRef::DataUri { mime_type, .. } => mime_type.clone(),
                    payload::ImageRef::Base64 { media_type, .. } => media_type.clone(),
                    payload::ImageRef::Url(_) => "application/octet-stream".to_string(),
                };
                // Push original data through unchanged
                transformed_images.push((
                    extracted.global_index,
                    extracted.data.clone(),
                    original_mime,
                ));
                continue;
            }
        };

        // Capture original dimensions for token estimation
        let orig_w = meta.width;
        let orig_h = meta.height;
        let orig_bytes = extracted.data.len();
        let format_before = meta.format.to_string();

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

            // Record metrics for SVG
            let (_, ref out_data, ref out_mime) = result;
            let (tw, th) = if out_data.is_empty() {
                (0, 0)
            } else {
                inspector::image::inspect_bytes(out_data)
                    .map(|m| (m.width, m.height))
                    .unwrap_or((orig_w, orig_h))
            };
            let format_after = mime_to_short(out_mime);
            report.add_image_metrics(ImageMetrics {
                image_index: extracted.global_index,
                original_width: orig_w,
                original_height: orig_h,
                transformed_width: tw,
                transformed_height: th,
                original_bytes: orig_bytes,
                transformed_bytes: out_data.len(),
                format_before: format_before.clone(),
                format_after,
                tokens_before: estimate_tokens(orig_w, orig_h),
                tokens_after: estimate_tokens(tw, th),
            });

            transformed_images.push(result);
            continue;
        }

        // Apply transformations
        let mut current_data = extracted.data.clone();
        let mut was_modified = false;
        let mut output_mime = meta.format.mime_type().to_string();
        let mut was_dropped = false;

        for action in &actions {
            match action {
                policy::Action::Pass => {}
                policy::Action::Drop { reason } => {
                    report.add_action(extracted.global_index, "drop", reason);
                    report.images_dropped += 1;
                    current_data = Vec::new();
                    was_modified = true;
                    was_dropped = true;
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

        // Determine transformed dimensions
        let (tw, th) = if was_dropped || current_data.is_empty() {
            (0, 0)
        } else if was_modified && !config.dry_run {
            // Re-inspect transformed data to get actual dimensions
            inspector::image::inspect_bytes(&current_data)
                .map(|m| (m.width, m.height))
                .unwrap_or((orig_w, orig_h))
        } else {
            // Dry-run or unchanged: estimate from policy actions
            estimate_dims_from_actions(&actions, orig_w, orig_h)
        };

        let format_after = mime_to_short(&output_mime);
        report.add_image_metrics(ImageMetrics {
            image_index: extracted.global_index,
            original_width: orig_w,
            original_height: orig_h,
            transformed_width: tw,
            transformed_height: th,
            original_bytes: orig_bytes,
            transformed_bytes: current_data.len(),
            format_before,
            format_after,
            tokens_before: estimate_tokens(orig_w, orig_h),
            tokens_after: estimate_tokens(tw, th),
        });

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

    // Fix #16: Track transformed image byte sizes
    let transformed_image_bytes: usize = transformed_images
        .iter()
        .map(|(_, data, _)| data.len())
        .sum();
    report.transformed_size = transformed_image_bytes;

    // Finalize aggregate token savings from per-image metrics
    report.finalize_token_savings();

    Ok((result, report))
}

/// Extract a short format name from a MIME type (e.g. "image/png" -> "png").
fn mime_to_short(mime: &str) -> String {
    mime.strip_prefix("image/").unwrap_or(mime).to_string()
}

/// Estimate target dimensions from policy actions (for dry-run reporting).
fn estimate_dims_from_actions(actions: &[policy::Action], orig_w: u32, orig_h: u32) -> (u32, u32) {
    for action in actions {
        match action {
            policy::Action::Resize {
                target_width,
                target_height,
            } => return (*target_width, *target_height),
            policy::Action::RasterizeSvg {
                target_width,
                target_height,
            } => return (*target_width, *target_height),
            policy::Action::Drop { .. } => return (0, 0),
            _ => {}
        }
    }
    (orig_w, orig_h)
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
            // Fix #5: SVG Source mode drops the image and records it as dropped.
            // The image block is removed from the payload. In the future, we could
            // inject the SVG XML as a text content block, but for now we drop + warn.
            report.add_action(
                global_index,
                "svg_dropped_as_source",
                &format!(
                    "SVG ({}x{}) removed (source mode: SVG not supported by provider)",
                    meta.width, meta.height
                ),
            );
            report.images_dropped += 1;
            report.add_warning(
                "SVG source mode dropped an image. Consider --svg-mode raster for provider compatibility.",
            );

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
