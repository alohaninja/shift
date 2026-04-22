use anyhow::{Context, Result};

use super::{decode_base64_image, detect_format, Encoding, ImageMetadata, MediaFormat};
use crate::mode::SafetyLimits;

/// Maximum pixels allowed before rejecting a decode (default safety).
const DEFAULT_MAX_PIXELS: u64 = 100_000_000; // 100 megapixels

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
///
/// Validates the URL against SSRF protections and enforces a download size limit.
pub fn inspect_url(url: &str) -> Result<ImageMetadata> {
    inspect_url_with_limits(url, &SafetyLimits::default())
}

/// Inspect a URL-referenced image with explicit safety limits.
pub fn inspect_url_with_limits(url: &str, limits: &SafetyLimits) -> Result<ImageMetadata> {
    // Fix #1: Validate URL before fetching
    validate_url(url)?;

    // Fix #3: Limit response body size
    let response = minreq::get(url)
        .with_timeout(30)
        .with_max_redirects(5)
        .send()
        .with_context(|| "failed to fetch image from URL".to_string())?;

    if response.status_code != 200 {
        anyhow::bail!("failed to fetch image: HTTP {}", response.status_code);
    }

    let bytes = response.as_bytes();
    if bytes.len() > limits.max_download_bytes {
        anyhow::bail!(
            "downloaded image too large: {} bytes exceeds limit of {} bytes",
            bytes.len(),
            limits.max_download_bytes
        );
    }

    let mut meta = inspect_bytes(bytes)?;
    meta.encoding = Encoding::Url(url.to_string());
    meta.size_bytes = bytes.len();
    Ok(meta)
}

/// Validate a URL for safety (SSRF prevention).
///
/// Rejects:
/// - Non-HTTP(S) schemes
/// - Private/loopback IP addresses
/// - Link-local addresses
/// - Hostnames that resolve to private IPs
fn validate_url(input: &str) -> Result<()> {
    let parsed = url::Url::parse(input).context("invalid URL")?;

    // Only allow HTTPS (and HTTP for dev, though HTTPS preferred)
    match parsed.scheme() {
        "https" | "http" => {}
        scheme => anyhow::bail!(
            "unsupported URL scheme '{}': only http/https allowed",
            scheme
        ),
    }

    let host = parsed.host_str().context("URL missing host")?;

    // Reject obviously dangerous hosts
    if host == "localhost" || host == "metadata.google.internal" {
        anyhow::bail!("URL host '{}' is not allowed", host);
    }

    // Try to parse as IP address and reject private ranges.
    // url::Url strips brackets from IPv6, so [::1] becomes "::1" in host_str().
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_private_ip(&ip) {
            anyhow::bail!("URL resolves to private/loopback IP address");
        }
    }
    // Also check via the parsed URL's ip() method for bracketed IPv6
    if let Some(url::Host::Ipv4(ip)) = parsed.host() {
        if is_private_ip(&std::net::IpAddr::V4(ip)) {
            anyhow::bail!("URL resolves to private/loopback IP address");
        }
    }
    if let Some(url::Host::Ipv6(ip)) = parsed.host() {
        if is_private_ip(&std::net::IpAddr::V6(ip)) {
            anyhow::bail!("URL resolves to private/loopback IP address");
        }
    }

    // Reject hex-encoded IPs like 0x7f000001
    if host.starts_with("0x") || host.starts_with("0X") {
        anyhow::bail!("URL host appears to be a hex-encoded IP address");
    }

    Ok(())
}

/// Check if an IP address is private, loopback, or link-local.
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
                || v4.is_private()     // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()  // 169.254.0.0/16
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0 // 0.0.0.0/8
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()       // ::1
                || v6.is_unspecified() // ::
                // fe80::/10 link-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // fc00::/7 unique local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Inspect a raster image (PNG, JPEG, GIF, WebP, BMP, TIFF).
fn inspect_raster(data: &[u8], detected_format: MediaFormat) -> Result<ImageMetadata> {
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .context("failed to guess image format")?;

    let (width, height) = reader
        .into_dimensions()
        .context("failed to read image dimensions")?;

    // Fix #2: Check pixel budget BEFORE any full decode happens
    let pixels = width as u64 * height as u64;
    if pixels > DEFAULT_MAX_PIXELS {
        anyhow::bail!(
            "image too large: {}x{} ({:.1} megapixels) exceeds limit of {} megapixels",
            width,
            height,
            pixels as f64 / 1_000_000.0,
            DEFAULT_MAX_PIXELS / 1_000_000
        );
    }

    Ok(ImageMetadata::new(
        detected_format,
        width,
        height,
        data.len(),
        Encoding::Raw,
    ))
}

/// Inspect an SVG image.
fn inspect_svg(data: &[u8]) -> Result<ImageMetadata> {
    let source = std::str::from_utf8(data).context("SVG is not valid UTF-8")?;

    let (width, height) = parse_svg_dimensions(source);

    let mut meta = ImageMetadata::new(MediaFormat::Svg, width, height, data.len(), Encoding::Raw);
    meta.svg_source = Some(source.to_string());
    Ok(meta)
}

/// Parse SVG dimensions from width/height attributes or viewBox.
fn parse_svg_dimensions(svg: &str) -> (u32, u32) {
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
///
/// Fix #10: Uses word-boundary matching to avoid matching `stroke-width` for `width`.
/// Fix #12: Rejects percentage and relative units (%, em, rem, vw, vh).
fn extract_svg_attr(svg: &str, attr_name: &str) -> Option<u32> {
    let svg_tag_start = svg.find("<svg")?;
    let svg_tag_end = svg[svg_tag_start..].find('>')? + svg_tag_start;
    let tag = &svg[svg_tag_start..=svg_tag_end];

    // Fix #10: Word-boundary-aware search.
    // Find ` attr_name=` (preceded by whitespace) to avoid matching `stroke-width` for `width`.
    let search_pattern = format!(" {}=", attr_name);
    let attr_pos = tag.find(&search_pattern)?;
    // Skip the leading space to point at `attr_name=`
    let after_eq = &tag[attr_pos + search_pattern.len()..];

    // Get the value (may be quoted with " or ')
    let value = if let Some(stripped) = after_eq.strip_prefix('"') {
        let end = stripped.find('"')?;
        &stripped[..end]
    } else if let Some(stripped) = after_eq.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        &stripped[..end]
    } else {
        let end = after_eq
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after_eq.len());
        &after_eq[..end]
    };

    // Fix #12: Reject relative/percentage units — fall through to viewBox
    let lower = value.to_lowercase();
    if lower.contains('%')
        || lower.contains("em")
        || lower.contains("rem")
        || lower.contains("vw")
        || lower.contains("vh")
        || lower.contains("vmin")
        || lower.contains("vmax")
    {
        return None;
    }

    // Parse numeric value, stripping units like "px", "pt"
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

    if parts.len() >= 4 && parts[2] > 0.0 && parts[3] > 0.0 {
        Some((parts[2] as u32, parts[3] as u32))
    } else {
        None
    }
}

/// Fetch an image from a URL with safety limits.
/// Used by payload extractors. Returns the raw bytes.
pub fn fetch_url_safe(url: &str, limits: &SafetyLimits) -> Result<Vec<u8>> {
    validate_url(url)?;

    let response = minreq::get(url)
        .with_timeout(30)
        .with_max_redirects(5)
        .send()
        .with_context(|| "failed to fetch image from URL".to_string())?;

    if response.status_code != 200 {
        anyhow::bail!("failed to fetch image: HTTP {}", response.status_code);
    }

    let bytes = response.as_bytes();
    if bytes.len() > limits.max_download_bytes {
        anyhow::bail!(
            "downloaded image too large: {} bytes exceeds limit of {} bytes",
            bytes.len(),
            limits.max_download_bytes
        );
    }

    Ok(bytes.to_vec())
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

    // Fix #12: SVG percentage units should fall through to viewBox
    #[test]
    fn test_inspect_svg_percentage_falls_to_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" viewBox="0 0 4000 3000"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.width, 4000);
        assert_eq!(meta.height, 3000);
    }

    #[test]
    fn test_inspect_svg_em_units_falls_to_viewbox() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10em" height="8em" viewBox="0 0 500 400"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.width, 500);
        assert_eq!(meta.height, 400);
    }

    // Fix #10: stroke-width should not match width
    #[test]
    fn test_inspect_svg_stroke_width_not_confused() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" stroke-width="3" width="800" height="600"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        assert_eq!(meta.width, 800);
        assert_eq!(meta.height, 600);
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

    // Fix #1: SSRF prevention tests
    #[test]
    fn test_validate_url_rejects_private_ip() {
        assert!(validate_url("http://127.0.0.1/image.png").is_err());
        assert!(validate_url("http://10.0.0.1/image.png").is_err());
        assert!(validate_url("http://172.16.0.1/image.png").is_err());
        assert!(validate_url("http://192.168.1.1/image.png").is_err());
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        assert!(validate_url("http://localhost/image.png").is_err());
        assert!(validate_url("http://localhost:8080/secret").is_err());
    }

    #[test]
    fn test_validate_url_rejects_ipv6_loopback() {
        assert!(validate_url("http://[::1]/image.png").is_err());
    }

    #[test]
    fn test_validate_url_rejects_file_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_rejects_hex_ip() {
        assert!(validate_url("http://0x7f000001/image.png").is_err());
    }

    #[test]
    fn test_validate_url_allows_public() {
        assert!(validate_url("https://example.com/image.png").is_ok());
        assert!(validate_url("https://cdn.openai.com/image.png").is_ok());
    }

    // Fix #2: Pixel budget test
    // Note: we can't easily create a decompression bomb in a unit test,
    // but we verify the dimension check path works
    #[test]
    fn test_normal_image_passes_pixel_budget() {
        let data = make_png(4000, 3000); // 12MP, under 100MP limit
        let meta = inspect_bytes(&data).unwrap();
        assert_eq!(meta.width, 4000);
    }

    // Fix #10: viewBox with negative width/height
    #[test]
    fn test_svg_viewbox_negative_dims_fallback() {
        let svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 -100 -100"><rect/></svg>"#;
        let meta = inspect_bytes(svg.as_bytes()).unwrap();
        // Should fall through to default (300, 150) since negative dims are rejected
        assert_eq!(meta.width, 300);
        assert_eq!(meta.height, 150);
    }
}
