use super::provider::ModelConstraints;
use crate::inspector::{ImageMetadata, MediaFormat};
use crate::mode::DriveMode;
use serde::{Deserialize, Serialize};

/// An action to be taken on an image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// No changes needed
    Pass,
    /// Resize to fit within max_dim (preserving aspect ratio)
    Resize {
        target_width: u32,
        target_height: u32,
    },
    /// Recompress at a given JPEG quality
    Recompress { quality: u8 },
    /// Convert from unsupported format to a safe one
    ConvertFormat { to: String },
    /// Rasterize SVG to PNG
    RasterizeSvg {
        target_width: u32,
        target_height: u32,
    },
    /// Drop this image entirely
    Drop { reason: String },
}

/// Evaluate what actions are needed for a single image.
pub fn evaluate(
    meta: &ImageMetadata,
    constraints: &ModelConstraints,
    mode: DriveMode,
    image_index: usize,
    total_images: usize,
) -> Vec<Action> {
    let mut actions = Vec::new();

    // 1. SVG handling — always needs conversion for provider safety
    if meta.format == MediaFormat::Svg {
        let (w, h) = svg_raster_dimensions(meta, constraints, mode);
        actions.push(Action::RasterizeSvg {
            target_width: w,
            target_height: h,
        });
        // After rasterization, the image is PNG — further checks apply to the rasterized output
        // but we can predict whether resizing will be needed based on the raster dimensions
        return actions;
    }

    // 2. Format conversion — BMP, TIFF, etc. need converting to provider-safe format
    if !meta.format.is_provider_safe() {
        actions.push(Action::ConvertFormat {
            to: "png".to_string(),
        });
    }

    // 3. Dimension checks
    let needs_resize = check_dimensions(meta, constraints, mode);
    if let Some((tw, th)) = needs_resize {
        actions.push(Action::Resize {
            target_width: tw,
            target_height: th,
        });
    }

    // 4. Megapixel check (Anthropic)
    if let Some(max_mp) = constraints.max_image_megapixels {
        if meta.megapixels > max_mp && needs_resize.is_none() {
            // Need to resize to fit megapixel limit
            let scale = (max_mp / meta.megapixels).sqrt();
            let tw = (meta.width as f64 * scale) as u32;
            let th = (meta.height as f64 * scale) as u32;
            actions.push(Action::Resize {
                target_width: tw,
                target_height: th,
            });
        }
    }

    // 5. File size check
    if meta.size_bytes > constraints.max_image_size_bytes {
        // Try recompression first (if JPEG)
        let quality = match mode {
            DriveMode::Performance => 90,
            DriveMode::Balanced => 80,
            DriveMode::Economy => 60,
        };
        actions.push(Action::Recompress { quality });
    }

    // 6. Economy mode: drop excess images
    if mode == DriveMode::Economy
        && total_images > constraints.max_images
        && image_index >= constraints.max_images
    {
        actions.clear();
        actions.push(Action::Drop {
            reason: format!(
                "economy mode: image {} exceeds max_images limit of {}",
                image_index + 1,
                constraints.max_images
            ),
        });
    }

    // 7. Mode-based aggressive resizing (economy)
    if mode == DriveMode::Economy
        && actions.iter().all(|a| matches!(a, Action::Pass))
        && meta.max_dim() > 1024
    {
        // In economy mode, aggressively downscale even if within limits
        let scale = 1024.0 / meta.max_dim() as f64;
        let tw = (meta.width as f64 * scale) as u32;
        let th = (meta.height as f64 * scale) as u32;
        actions.push(Action::Resize {
            target_width: tw,
            target_height: th,
        });
    }

    // If no actions were added, it's a pass
    if actions.is_empty() {
        actions.push(Action::Pass);
    }

    actions
}

/// Determine resize dimensions based on constraints and mode.
fn check_dimensions(
    meta: &ImageMetadata,
    constraints: &ModelConstraints,
    mode: DriveMode,
) -> Option<(u32, u32)> {
    let max_dim = match mode {
        DriveMode::Performance => constraints.max_image_dim,
        DriveMode::Balanced => constraints.max_image_dim,
        DriveMode::Economy => constraints.max_image_dim.min(1024),
    };

    if meta.max_dim() > max_dim {
        let scale = max_dim as f64 / meta.max_dim() as f64;
        let tw = (meta.width as f64 * scale) as u32;
        let th = (meta.height as f64 * scale) as u32;
        Some((tw.max(1), th.max(1)))
    } else {
        None
    }
}

/// Determine rasterization dimensions for SVG.
fn svg_raster_dimensions(
    meta: &ImageMetadata,
    constraints: &ModelConstraints,
    mode: DriveMode,
) -> (u32, u32) {
    let max_target = match mode {
        DriveMode::Performance => constraints.max_image_dim.min(2048),
        DriveMode::Balanced => constraints.max_image_dim.min(1024),
        DriveMode::Economy => 512,
    };

    let w = meta.width;
    let h = meta.height;

    if w == 0 || h == 0 {
        return (max_target, max_target);
    }

    if w.max(h) > max_target {
        let scale = max_target as f64 / w.max(h) as f64;
        let tw = (w as f64 * scale) as u32;
        let th = (h as f64 * scale) as u32;
        (tw.max(1), th.max(1))
    } else if w.max(h) < 64 {
        // Very small SVG: scale up to at least 256px
        let scale = 256.0 / w.max(h) as f64;
        let tw = (w as f64 * scale) as u32;
        let th = (h as f64 * scale) as u32;
        (tw, th)
    } else {
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::Encoding;

    fn make_constraints() -> ModelConstraints {
        ModelConstraints {
            max_images: 10,
            max_image_dim: 2048,
            max_image_size_bytes: 20_971_520,
            max_image_megapixels: None,
            supported_formats: vec!["png".into(), "jpeg".into(), "gif".into(), "webp".into()],
        }
    }

    fn make_anthropic_constraints() -> ModelConstraints {
        ModelConstraints {
            max_images: 20,
            max_image_dim: 8000,
            max_image_size_bytes: 5_242_880,
            max_image_megapixels: Some(1.15),
            supported_formats: vec!["png".into(), "jpeg".into(), "gif".into(), "webp".into()],
        }
    }

    fn make_meta(format: MediaFormat, w: u32, h: u32, size: usize) -> ImageMetadata {
        ImageMetadata::new(format, w, h, size, Encoding::Base64)
    }

    #[test]
    fn test_pass_small_png() {
        let meta = make_meta(MediaFormat::Png, 640, 480, 50_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Balanced, 0, 1);
        assert_eq!(actions, vec![Action::Pass]);
    }

    #[test]
    fn test_resize_oversized_image() {
        let meta = make_meta(MediaFormat::Png, 4000, 3000, 100_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Balanced, 0, 1);
        assert!(actions.iter().any(|a| matches!(a, Action::Resize { .. })));
        if let Action::Resize {
            target_width,
            target_height,
        } = &actions[0]
        {
            assert!(*target_width <= 2048);
            assert!(*target_height <= 2048);
        }
    }

    #[test]
    fn test_resize_performance_mode_only_if_over_limit() {
        // 2000px is under 2048 limit — performance mode should pass
        let meta = make_meta(MediaFormat::Png, 2000, 1500, 100_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Performance, 0, 1);
        assert_eq!(actions, vec![Action::Pass]);
    }

    #[test]
    fn test_economy_mode_aggressive_resize() {
        // 1500px is under 2048 but economy mode caps at 1024
        let meta = make_meta(MediaFormat::Png, 1500, 1000, 100_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Economy, 0, 1);
        assert!(actions.iter().any(|a| matches!(a, Action::Resize { .. })));
    }

    #[test]
    fn test_economy_mode_drops_excess_images() {
        let meta = make_meta(MediaFormat::Png, 640, 480, 50_000);
        let constraints = make_constraints(); // max 10 images
        let actions = evaluate(&meta, &constraints, DriveMode::Economy, 10, 11);
        assert!(actions.iter().any(|a| matches!(a, Action::Drop { .. })));
    }

    #[test]
    fn test_svg_rasterized() {
        let mut meta = make_meta(MediaFormat::Svg, 800, 600, 5_000);
        meta.svg_source = Some("<svg></svg>".to_string());
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Balanced, 0, 1);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::RasterizeSvg { .. })));
    }

    #[test]
    fn test_bmp_converted() {
        let meta = make_meta(MediaFormat::Bmp, 640, 480, 900_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Balanced, 0, 1);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::ConvertFormat { .. })));
    }

    #[test]
    fn test_anthropic_megapixel_limit() {
        // 2000x1000 = 2.0 MP, over 1.15 MP limit
        let meta = make_meta(MediaFormat::Png, 2000, 1000, 100_000);
        let actions = evaluate(
            &meta,
            &make_anthropic_constraints(),
            DriveMode::Balanced,
            0,
            1,
        );
        assert!(actions.iter().any(|a| matches!(a, Action::Resize { .. })));
    }

    #[test]
    fn test_anthropic_under_megapixel_limit() {
        // 1000x800 = 0.8 MP, under 1.15 MP limit
        let meta = make_meta(MediaFormat::Png, 1000, 800, 100_000);
        let actions = evaluate(
            &meta,
            &make_anthropic_constraints(),
            DriveMode::Balanced,
            0,
            1,
        );
        assert_eq!(actions, vec![Action::Pass]);
    }

    #[test]
    fn test_oversized_file_recompressed() {
        // 25 MB file, over 20 MB limit
        let meta = make_meta(MediaFormat::Jpeg, 1000, 800, 25_000_000);
        let actions = evaluate(&meta, &make_constraints(), DriveMode::Balanced, 0, 1);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::Recompress { quality: 80 })));
    }

    #[test]
    fn test_recompress_quality_by_mode() {
        let meta = make_meta(MediaFormat::Jpeg, 1000, 800, 25_000_000);
        let constraints = make_constraints();

        let perf_actions = evaluate(&meta, &constraints, DriveMode::Performance, 0, 1);
        assert!(perf_actions
            .iter()
            .any(|a| matches!(a, Action::Recompress { quality: 90 })));

        let eco_actions = evaluate(&meta, &constraints, DriveMode::Economy, 0, 1);
        assert!(eco_actions
            .iter()
            .any(|a| matches!(a, Action::Recompress { quality: 60 })));
    }
}
