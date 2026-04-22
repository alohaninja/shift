use anyhow::{Context, Result};

use super::{decode_base64_image, detect_format, Encoding, ImageMetadata, MediaFormat};

/// Inspect raw image bytes and extract metadata.
pub fn inspect_bytes(data: &[u8]) -> Result<ImageMetadata> {
    let format = detect_format(data);

    match format {
        MediaFormat::Svg => inspect_svg(data),
        _ if format.is_image() => inspect_raster(data, format),
        _ => anyhow::bail!("not a recognized image format"),
    }
}

/// Inspect a base64-encoded image (data URI or raw base64).
pub fn inspect_base64(input: &str) -> Result<ImageMetadata> {
    let (bytes, _mime_hint) = decode_base64_image(input)?;
    let mut meta = inspect_bytes(&bytes)?;
    meta.encoding = Encoding::Base64;
    meta.size_bytes = bytes.len(); // decoded size
    Ok(meta)
}

/// Inspect an image referenced by URL (fetches it).
pub fn inspect_url(url: &str) -> Result<ImageMetadata> {
    let response = minreq::get(url)
        .with_timeout(30)
        .send()
        .with_context(|| format!("failed to fetch image from {}", url))?;

    if response.status_code != 200 {
        anyhow::bail!(
            "failed to fetch image from {}: HTTP {}",
            url,
            response.status_code
        );
    }

    let bytes = response.as_bytes();
    let mut meta = inspect_bytes(bytes)?;
    meta.encoding = Encoding::Url(url.to_string());
    meta.size_bytes = bytes.len();
    Ok(meta)
}

/// Inspect a raster image (PNG, JPEG, GIF, WebP, BMP, TIFF).
fn inspect_raster(data: &[u8], detected_format: MediaFormat) -> Result<ImageMetadata> {
    // Use the `image` crate to get dimensions without fully decoding
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?;

    let (width, height) = reader
        .into_dimensions()
        .context("failed to read image dimensions")?;

    Ok(ImageMetadata::new(
        detected_format,
        width,
        height,
        data.len(),
        Encoding::Raw,
    ))
}

/// Inspect an SVG image.
///
/// Extracts the viewBox or width/height attributes to determine dimensions.
/// Stores the SVG source for potential rasterization.
fn inspect_svg(data: &[u8]) -> Result<ImageMetadata> {
    let source = std::str::from_utf8(data).context("SVG is not valid UTF-8")?;

    let (width, height) = parse_svg_dimensions(source);

    let mut meta = ImageMetadata::new(MediaFormat::Svg, width, height, data.len(), Encoding::Raw);
    meta.svg_source = Some(source.to_string());
    Ok(meta)
}

/// Parse SVG dimensions from width/height attributes or viewBox.
fn parse_svg_dimensions(svg: &str) -> (u32, u32) {
    // Try to extract from <svg> tag attributes
    // Look for width="..." and height="..."
    let width = extract_svg_attr(svg, "width");
    let height = extract_svg_attr(svg, "height");

    if let (Some(w), Some(h)) = (width, height) {
        if w > 0 && h > 0 {
            return (w, h);
        }
    }

    // Fall back to viewBox
    if let Some(vb) = extract_svg_viewbox(svg) {
        return vb;
    }

    // Default fallback
    (300, 150)
}

/// Extract a numeric attribute value from the <svg> tag.
fn extract_svg_attr(svg: &str, attr_name: &str) -> Option<u32> {
    // Find the <svg tag
    let svg_tag_start = svg.find("<svg")?;
    let svg_tag_end = svg[svg_tag_start..].find('>')? + svg_tag_start;
    let tag = &svg[svg_tag_start..=svg_tag_end];

    // Find attr_name="value"
    let pattern = format!("{}=", attr_name);
    let attr_pos = tag.find(&pattern)?;
    let after_eq = &tag[attr_pos + pattern.len()..];

    // Get the value (may be quoted with " or ')
    let value = if let Some(stripped) = after_eq.strip_prefix('"') {
        let end = stripped.find('"')?;
        &stripped[..end]
    } else if let Some(stripped) = after_eq.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        &stripped[..end]
    } else {
        // Unquoted — grab until whitespace or >
        let end = after_eq
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after_eq.len());
        &after_eq[..end]
    };

    // Parse numeric value, stripping units like "px", "pt", "em"
    let numeric: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    numeric.parse::<f64>().ok().map(|v| v as u32)
}

/// Extract viewBox dimensions (returns width, height from the viewBox).
fn extract_svg_viewbox(svg: &str) -> Option<(u32, u32)> {
    let svg_tag_start = svg.find("<svg")?;
    let svg_tag_end = svg[svg_tag_start..].find('>')? + svg_tag_start;
    let tag = &svg[svg_tag_start..=svg_tag_end];

    let vb_pos = tag.find("viewBox=")?;
    let after_eq = &tag[vb_pos + 8..];

    let value = if let Some(stripped) = after_eq.strip_prefix('"') {
        let end = stripped.find('"')?;
        &stripped[..end]
    } else if let Some(stripped) = after_eq.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        &stripped[..end]
    } else {
        return None;
    };

    // viewBox="minX minY width height"
    let parts: Vec<f64> = value
        .split_whitespace()
        .flat_map(|s| s.split(','))
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    if parts.len() >= 4 {
        Some((parts[2] as u32, parts[3] as u32))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_png(width: u32, height: u32) -> Vec<u8> {
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
        buf
    }

    fn make_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::new(width, height);
        let mut buf = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
        encoder
            .encode(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        buf
    }

    #[test]
    fn test_inspect_png() {
        let data = make_png(640, 480);
        let meta = inspect_bytes(&data).unwrap();
        assert_eq!(meta.format, MediaFormat::Png);
        assert_eq!(meta.width, 640);
        assert_eq!(meta.height, 480);
        assert_eq!(meta.max_dim(), 640);
    }

    #[test]
    fn test_inspect_jpeg() {
        let data = make_jpeg(1920, 1080);
        let meta = inspect_bytes(&data).unwrap();
        assert_eq!(meta.format, MediaFormat::Jpeg);
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
    }

    #[test]
    fn test_inspect_svg_with_dimensions() {
        let svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.format, MediaFormat::Svg);
        assert_eq!(meta.width, 200);
        assert_eq!(meta.height, 100);
        assert!(meta.svg_source.is_some());
    }

    #[test]
    fn test_inspect_svg_with_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.format, MediaFormat::Svg);
        assert_eq!(meta.width, 800);
        assert_eq!(meta.height, 600);
    }

    #[test]
    fn test_inspect_svg_with_xml_declaration() {
        let svg = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="500" height="300">
  <circle cx="250" cy="150" r="100"/>
</svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.format, MediaFormat::Svg);
        assert_eq!(meta.width, 500);
        assert_eq!(meta.height, 300);
    }

    #[test]
    fn test_inspect_svg_viewbox_comma_separated() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0,0,1024,768"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.width, 1024);
        assert_eq!(meta.height, 768);
    }

    #[test]
    fn test_inspect_svg_px_units() {
        let svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200px" height="150px"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.width, 200);
        assert_eq!(meta.height, 150);
    }

    #[test]
    fn test_inspect_base64_png() {
        use base64::Engine;
        let png_data = make_png(100, 50);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png_data);
        let data_uri = format!("data:image/png;base64,{}", encoded);

        let meta = inspect_base64(&data_uri).unwrap();
        assert_eq!(meta.format, MediaFormat::Png);
        assert_eq!(meta.width, 100);
        assert_eq!(meta.height, 50);
        assert_eq!(meta.encoding, Encoding::Base64);
    }

    #[test]
    fn test_inspect_base64_raw() {
        use base64::Engine;
        let png_data = make_png(64, 64);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png_data);

        let meta = inspect_base64(&encoded).unwrap();
        assert_eq!(meta.format, MediaFormat::Png);
        assert_eq!(meta.width, 64);
        assert_eq!(meta.height, 64);
    }

    #[test]
    fn test_inspect_not_an_image() {
        let result = inspect_bytes(b"this is just text, not an image");
        assert!(result.is_err());
    }

    #[test]
    fn test_megapixels() {
        let data = make_png(4000, 3000);
        let meta = inspect_bytes(&data).unwrap();
        assert!((meta.megapixels - 12.0).abs() < 0.001);
    }
}
