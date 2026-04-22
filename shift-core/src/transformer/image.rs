use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageEncoder};

use crate::policy::Action;

/// Apply a transformation action to raw image bytes.
///
/// Returns the transformed image bytes.
pub fn transform_image(data: &[u8], action: &Action) -> Result<Vec<u8>> {
    match action {
        Action::Pass => Ok(data.to_vec()),

        Action::Resize {
            target_width,
            target_height,
        } => resize_image(data, *target_width, *target_height),

        Action::Recompress { quality } => recompress_jpeg(data, *quality),

        Action::ConvertFormat { to } => convert_format(data, to),

        Action::RasterizeSvg {
            target_width,
            target_height,
        } => {
            // SVG data should be passed as the raw SVG text bytes
            let svg_text = std::str::from_utf8(data).context("SVG data is not valid UTF-8")?;
            rasterize_svg(svg_text, *target_width, *target_height)
        }

        Action::Drop { .. } => {
            // Dropping returns empty — caller handles removal from payload
            Ok(Vec::new())
        }
    }
}

/// Load an image from memory with a pixel budget to prevent decompression bombs.
///
/// R5: Propagates dimension-read errors instead of silently falling through
/// to an unguarded decode. If we can't read dimensions from the header,
/// we reject the image rather than risk a decompression bomb.
fn load_image_safe(data: &[u8]) -> Result<DynamicImage> {
    use crate::mode::SafetyLimits;

    let limits = SafetyLimits::default();

    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?;

    // R5: Propagate the error — don't silently skip the budget check
    let (w, h) = reader
        .into_dimensions()
        .context("failed to read image dimensions (cannot verify pixel budget)")?;

    let pixels = w as u64 * h as u64;
    if pixels > limits.max_pixels {
        anyhow::bail!(
            "image decompression blocked: {}x{} ({:.0} megapixels) exceeds {:.0} megapixel safety limit",
            w,
            h,
            pixels as f64 / 1_000_000.0,
            limits.max_pixels as f64 / 1_000_000.0
        );
    }

    // Now do the full decode — we know it's within pixel budget
    image::load_from_memory(data).context("failed to decode image")
}

/// Resize an image to fit within target dimensions, preserving aspect ratio.
fn resize_image(data: &[u8], target_width: u32, target_height: u32) -> Result<Vec<u8>> {
    let img = load_image_safe(data)?;

    let resized = img.resize(target_width, target_height, FilterType::Lanczos3);

    // Encode as PNG (lossless, safe for all providers)
    encode_png(&resized)
}

/// Recompress an image as JPEG at the given quality.
fn recompress_jpeg(data: &[u8], quality: u8) -> Result<Vec<u8>> {
    let img = load_image_safe(data)?;

    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    encoder
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("failed to encode JPEG")?;

    Ok(buf)
}

/// Convert an image to a different format.
fn convert_format(data: &[u8], to: &str) -> Result<Vec<u8>> {
    let img = load_image_safe(data)?;

    match to {
        "png" => encode_png(&img),
        "jpeg" | "jpg" => {
            let rgb = img.to_rgb8();
            let mut buf = Vec::new();
            let encoder = JpegEncoder::new_with_quality(&mut buf, 85);
            encoder
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .context("failed to encode JPEG")?;
            Ok(buf)
        }
        _ => anyhow::bail!("unsupported target format: {}", to),
    }
}

/// Encode a DynamicImage as PNG.
fn encode_png(img: &DynamicImage) -> Result<Vec<u8>> {
    let rgba = img.to_rgba8();
    let mut buf = Vec::new();
    let encoder = PngEncoder::new(&mut buf);
    encoder
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .context("failed to encode PNG")?;
    Ok(buf)
}

/// Rasterize SVG text to PNG at the given dimensions.
pub fn rasterize_svg(svg_text: &str, target_width: u32, target_height: u32) -> Result<Vec<u8>> {
    use resvg::tiny_skia;
    use resvg::usvg;

    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg_text, &options).context("failed to parse SVG")?;

    let size = tree.size();
    let (svg_w, svg_h) = (size.width(), size.height());

    // Calculate scale to fit within target dimensions
    let scale_x = target_width as f32 / svg_w;
    let scale_y = target_height as f32 / svg_h;
    let scale = scale_x.min(scale_y);

    let pixel_w = (svg_w * scale).ceil() as u32;
    let pixel_h = (svg_h * scale).ceil() as u32;

    // R9: Pixel budget for SVG rasterization (shared via SafetyLimits)
    let limits = crate::mode::SafetyLimits::default();
    let pixel_count = pixel_w as u64 * pixel_h as u64;
    if pixel_count > limits.max_pixels {
        anyhow::bail!(
            "SVG rasterization blocked: {}x{} exceeds {:.0} megapixel safety limit",
            pixel_w,
            pixel_h,
            limits.max_pixels as f64 / 1_000_000.0
        );
    }

    let mut pixmap = tiny_skia::Pixmap::new(pixel_w.max(1), pixel_h.max(1))
        .context("failed to create pixmap")?;

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Convert pixmap to PNG
    let png_data = pixmap
        .encode_png()
        .context("failed to encode rasterized SVG as PNG")?;
    Ok(png_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::{detect_format, MediaFormat};

    fn make_test_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::new(width, height);
        let mut buf = Vec::new();
        let encoder = PngEncoder::new(&mut buf);
        encoder
            .write_image(img.as_raw(), width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn make_test_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::new(width, height);
        let mut buf = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut buf, 90);
        encoder
            .encode(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        buf
    }

    #[test]
    fn test_resize_png() {
        let data = make_test_png(4000, 3000);
        let action = Action::Resize {
            target_width: 2048,
            target_height: 2048,
        };
        let result = transform_image(&data, &action).unwrap();

        // Verify it's still a valid image
        let img = image::load_from_memory(&result).unwrap();
        assert!(img.width() <= 2048);
        assert!(img.height() <= 2048);
        // Verify aspect ratio preserved
        let ratio_orig = 4000.0 / 3000.0;
        let ratio_new = img.width() as f64 / img.height() as f64;
        assert!((ratio_orig - ratio_new).abs() < 0.02);
    }

    #[test]
    fn test_resize_preserves_format_as_png() {
        let data = make_test_png(3000, 2000);
        let action = Action::Resize {
            target_width: 1024,
            target_height: 1024,
        };
        let result = transform_image(&data, &action).unwrap();
        assert_eq!(detect_format(&result), MediaFormat::Png);
    }

    #[test]
    fn test_recompress_jpeg() {
        let data = make_test_jpeg(1000, 800);
        let original_size = data.len();
        let action = Action::Recompress { quality: 50 };
        let result = transform_image(&data, &action).unwrap();

        // Lower quality should produce a smaller file
        assert!(result.len() <= original_size);
        // Should still be valid JPEG
        assert_eq!(detect_format(&result), MediaFormat::Jpeg);
    }

    #[test]
    fn test_convert_bmp_to_png() {
        // Create a test image and save as BMP bytes
        let img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 0, 0]));
        let mut bmp_data = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bmp_data);
        img.write_to(&mut cursor, image::ImageFormat::Bmp).unwrap();

        let action = Action::ConvertFormat {
            to: "png".to_string(),
        };
        let result = transform_image(&bmp_data, &action).unwrap();
        assert_eq!(detect_format(&result), MediaFormat::Png);
    }

    #[test]
    fn test_pass_action() {
        let data = make_test_png(100, 100);
        let action = Action::Pass;
        let result = transform_image(&data, &action).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_drop_action() {
        let data = make_test_png(100, 100);
        let action = Action::Drop {
            reason: "test".into(),
        };
        let result = transform_image(&data, &action).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_rasterize_svg_simple() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <rect width="200" height="100" fill="red"/>
        </svg>"#;

        let result = rasterize_svg(svg, 200, 100).unwrap();
        assert!(!result.is_empty());
        assert_eq!(detect_format(&result), MediaFormat::Png);

        // Verify dimensions
        let img = image::load_from_memory(&result).unwrap();
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 100);
    }

    #[test]
    fn test_rasterize_svg_scaled_down() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="1000">
            <circle cx="1000" cy="500" r="400" fill="blue"/>
        </svg>"#;

        let result = rasterize_svg(svg, 500, 500).unwrap();
        let img = image::load_from_memory(&result).unwrap();
        assert!(img.width() <= 500);
        assert!(img.height() <= 500);
    }

    #[test]
    fn test_rasterize_svg_with_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" fill="green"/>
        </svg>"#;

        let result = rasterize_svg(svg, 256, 256).unwrap();
        assert!(!result.is_empty());
        assert_eq!(detect_format(&result), MediaFormat::Png);
    }

    #[test]
    fn test_rasterize_svg_complex() {
        let svg = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:rgb(255,0,0);stop-opacity:1" />
      <stop offset="100%" style="stop-color:rgb(0,0,255);stop-opacity:1" />
    </linearGradient>
  </defs>
  <rect width="400" height="300" fill="url(#grad)"/>
  <circle cx="200" cy="150" r="80" fill="white" opacity="0.5"/>
  <text x="200" y="160" text-anchor="middle" font-size="24" fill="white">SHIFT</text>
</svg>"#;

        let result = rasterize_svg(svg, 800, 600).unwrap();
        assert!(!result.is_empty());
        let img = image::load_from_memory(&result).unwrap();
        assert!(img.width() > 0);
        assert!(img.height() > 0);
    }

    #[test]
    fn test_transform_svg_via_action() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect width="100" height="100" fill="red"/>
        </svg>"#;

        let action = Action::RasterizeSvg {
            target_width: 256,
            target_height: 256,
        };
        let result = transform_image(svg.as_bytes(), &action).unwrap();
        assert_eq!(detect_format(&result), MediaFormat::Png);
    }
}
