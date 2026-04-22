pub mod image;

// v2 modality stubs
pub mod audio;
pub mod document;
pub mod video;

use serde::{Deserialize, Serialize};

use crate::mode::SafetyLimits;

/// Detected format of a media input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Svg,
    Bmp,
    Tiff,
    // Future
    Mp4,
    Mp3,
    Wav,
    Pdf,
    Unknown,
}

impl MediaFormat {
    /// Returns the MIME type string for this format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            MediaFormat::Png => "image/png",
            MediaFormat::Jpeg => "image/jpeg",
            MediaFormat::Gif => "image/gif",
            MediaFormat::WebP => "image/webp",
            MediaFormat::Svg => "image/svg+xml",
            MediaFormat::Bmp => "image/bmp",
            MediaFormat::Tiff => "image/tiff",
            MediaFormat::Mp4 => "video/mp4",
            MediaFormat::Mp3 => "audio/mpeg",
            MediaFormat::Wav => "audio/wav",
            MediaFormat::Pdf => "application/pdf",
            MediaFormat::Unknown => "application/octet-stream",
        }
    }

    /// Whether this format is a raster image supported by most providers.
    pub fn is_provider_safe(&self) -> bool {
        matches!(
            self,
            MediaFormat::Png | MediaFormat::Jpeg | MediaFormat::Gif | MediaFormat::WebP
        )
    }

    /// Whether this is an image format (raster or vector).
    pub fn is_image(&self) -> bool {
        matches!(
            self,
            MediaFormat::Png
                | MediaFormat::Jpeg
                | MediaFormat::Gif
                | MediaFormat::WebP
                | MediaFormat::Svg
                | MediaFormat::Bmp
                | MediaFormat::Tiff
        )
    }
}

impl std::fmt::Display for MediaFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaFormat::Png => write!(f, "png"),
            MediaFormat::Jpeg => write!(f, "jpeg"),
            MediaFormat::Gif => write!(f, "gif"),
            MediaFormat::WebP => write!(f, "webp"),
            MediaFormat::Svg => write!(f, "svg"),
            MediaFormat::Bmp => write!(f, "bmp"),
            MediaFormat::Tiff => write!(f, "tiff"),
            MediaFormat::Mp4 => write!(f, "mp4"),
            MediaFormat::Mp3 => write!(f, "mp3"),
            MediaFormat::Wav => write!(f, "wav"),
            MediaFormat::Pdf => write!(f, "pdf"),
            MediaFormat::Unknown => write!(f, "unknown"),
        }
    }
}

/// How the image is encoded in the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Base64-encoded inline data (data: URI or raw base64)
    Base64,
    /// URL reference (https://...)
    Url(String),
    /// Raw bytes (not from a payload, e.g. from file)
    Raw,
}

/// Metadata extracted from inspecting a media input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub format: MediaFormat,
    pub width: u32,
    pub height: u32,
    pub size_bytes: usize,
    pub encoding: Encoding,
    /// Megapixels (width * height / 1_000_000)
    pub megapixels: f64,
    /// For SVG: the raw SVG source text
    pub svg_source: Option<String>,
}

impl ImageMetadata {
    pub fn new(
        format: MediaFormat,
        width: u32,
        height: u32,
        size_bytes: usize,
        encoding: Encoding,
    ) -> Self {
        let megapixels = (width as f64 * height as f64) / 1_000_000.0;
        ImageMetadata {
            format,
            width,
            height,
            size_bytes,
            encoding,
            megapixels,
            svg_source: None,
        }
    }

    /// The larger dimension.
    pub fn max_dim(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// Detect format from raw bytes using magic bytes.
pub fn detect_format(data: &[u8]) -> MediaFormat {
    if data.len() < 4 {
        return MediaFormat::Unknown;
    }

    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return MediaFormat::Png;
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return MediaFormat::Jpeg;
    }

    // GIF: GIF87a or GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return MediaFormat::Gif;
    }

    // WebP: RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return MediaFormat::WebP;
    }

    // BMP: BM
    if data.starts_with(b"BM") {
        return MediaFormat::Bmp;
    }

    // TIFF: II or MM
    if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return MediaFormat::Tiff;
    }

    // SVG: look for XML/SVG markers in text
    if is_svg(data) {
        return MediaFormat::Svg;
    }

    // PDF: %PDF
    if data.starts_with(b"%PDF") {
        return MediaFormat::Pdf;
    }

    MediaFormat::Unknown
}

/// Check if data looks like SVG (XML with <svg element).
fn is_svg(data: &[u8]) -> bool {
    // Try to interpret as UTF-8 text
    let text = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => {
            // Try just the first 1KB
            let end = data.len().min(1024);
            match std::str::from_utf8(&data[..end]) {
                Ok(s) => s,
                Err(_) => return false,
            }
        }
    };

    let trimmed = text.trim_start();
    // XML declaration or <svg tag
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<svg") {
        // Must contain <svg somewhere
        return trimmed.contains("<svg");
    }

    false
}

/// Decode a base64 data URI or raw base64 string to bytes.
///
/// Handles formats:
/// - `data:image/png;base64,iVBOR...`
/// - `iVBOR...` (raw base64)
///
/// Enforces a size limit (default 30 MB base64 input) to prevent OOM.
/// Uses a tolerant decoder that accepts both padded and unpadded base64.
pub fn decode_base64_image(input: &str) -> anyhow::Result<(Vec<u8>, Option<String>)> {
    decode_base64_image_with_limits(input, &SafetyLimits::default())
}

/// Decode base64 with explicit safety limits.
pub fn decode_base64_image_with_limits(
    input: &str,
    limits: &SafetyLimits,
) -> anyhow::Result<(Vec<u8>, Option<String>)> {
    use base64::engine::general_purpose;
    use base64::Engine;

    let (b64_data, mime_hint) = if let Some(rest) = input.strip_prefix("data:") {
        // data:image/png;base64,iVBOR...
        if let Some(comma_pos) = rest.find(',') {
            let header = &rest[..comma_pos];
            let data = &rest[comma_pos + 1..];
            let mime = header.split(';').next().map(|s| s.to_string());
            (data, mime)
        } else {
            (rest, None)
        }
    } else {
        (input, None)
    };

    // Fix #9: Check base64 input size before allocating
    if b64_data.len() > limits.max_base64_bytes {
        anyhow::bail!(
            "base64 input too large: {} bytes exceeds limit of {} bytes",
            b64_data.len(),
            limits.max_base64_bytes
        );
    }

    // Fix #23: Use tolerant engine that accepts padded and unpadded base64
    let engine = general_purpose::STANDARD;

    // Strip whitespace/newlines from base64
    let cleaned: String = b64_data.chars().filter(|c| !c.is_whitespace()).collect();

    // Try standard (padded) first, then no-pad
    let bytes = engine
        .decode(&cleaned)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(&cleaned))?;

    Ok((bytes, mime_hint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_format(&data), MediaFormat::Png);
    }

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_format(&data), MediaFormat::Jpeg);
    }

    #[test]
    fn test_detect_gif() {
        assert_eq!(detect_format(b"GIF89a..."), MediaFormat::Gif);
        assert_eq!(detect_format(b"GIF87a..."), MediaFormat::Gif);
    }

    #[test]
    fn test_detect_webp() {
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&[0x00; 4]); // size placeholder
        data.extend_from_slice(b"WEBP");
        assert_eq!(detect_format(&data), MediaFormat::WebP);
    }

    #[test]
    fn test_detect_bmp() {
        let data = b"BM\x00\x00\x00\x00";
        assert_eq!(detect_format(data), MediaFormat::Bmp);
    }

    #[test]
    fn test_detect_svg_with_xml_declaration() {
        let data =
            b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>";
        assert_eq!(detect_format(data), MediaFormat::Svg);
    }

    #[test]
    fn test_detect_svg_bare() {
        let data = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><circle/></svg>";
        assert_eq!(detect_format(data), MediaFormat::Svg);
    }

    #[test]
    fn test_detect_svg_with_whitespace() {
        let data = b"  \n  <svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>";
        assert_eq!(detect_format(data), MediaFormat::Svg);
    }

    #[test]
    fn test_detect_pdf() {
        assert_eq!(detect_format(b"%PDF-1.4 ..."), MediaFormat::Pdf);
    }

    #[test]
    fn test_detect_unknown() {
        assert_eq!(detect_format(b"random data here"), MediaFormat::Unknown);
    }

    #[test]
    fn test_detect_too_short() {
        assert_eq!(detect_format(b"ab"), MediaFormat::Unknown);
    }

    #[test]
    fn test_media_format_mime() {
        assert_eq!(MediaFormat::Png.mime_type(), "image/png");
        assert_eq!(MediaFormat::Jpeg.mime_type(), "image/jpeg");
        assert_eq!(MediaFormat::Svg.mime_type(), "image/svg+xml");
    }

    #[test]
    fn test_media_format_is_provider_safe() {
        assert!(MediaFormat::Png.is_provider_safe());
        assert!(MediaFormat::Jpeg.is_provider_safe());
        assert!(MediaFormat::Gif.is_provider_safe());
        assert!(MediaFormat::WebP.is_provider_safe());
        assert!(!MediaFormat::Svg.is_provider_safe());
        assert!(!MediaFormat::Bmp.is_provider_safe());
        assert!(!MediaFormat::Tiff.is_provider_safe());
    }

    #[test]
    fn test_media_format_is_image() {
        assert!(MediaFormat::Png.is_image());
        assert!(MediaFormat::Svg.is_image());
        assert!(!MediaFormat::Mp4.is_image());
        assert!(!MediaFormat::Pdf.is_image());
    }

    #[test]
    fn test_decode_base64_data_uri() {
        use base64::Engine;
        let raw = vec![0x89, 0x50, 0x4E, 0x47]; // PNG header
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
        let uri = format!("data:image/png;base64,{}", encoded);

        let (bytes, mime) = decode_base64_image(&uri).unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(mime.unwrap(), "image/png");
    }

    #[test]
    fn test_decode_base64_raw() {
        use base64::Engine;
        let raw = vec![0xFF, 0xD8, 0xFF]; // JPEG header
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);

        let (bytes, mime) = decode_base64_image(&encoded).unwrap();
        assert_eq!(bytes, raw);
        assert!(mime.is_none());
    }

    #[test]
    fn test_decode_base64_size_limit() {
        let limits = SafetyLimits {
            max_base64_bytes: 100,
            ..Default::default()
        };
        let big_input = "A".repeat(200);
        let result = decode_base64_image_with_limits(&big_input, &limits);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_decode_base64_unpadded() {
        use base64::Engine;
        let raw = vec![0x89, 0x50, 0x4E, 0x47, 0x0D]; // 5 bytes
                                                      // Standard encoding would be "iVBORQ==" but no-pad is "iVBORQ"
        let encoded_nopad = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&raw);
        assert!(!encoded_nopad.contains('='));

        let (bytes, _) = decode_base64_image(&encoded_nopad).unwrap();
        assert_eq!(bytes, raw);
    }

    #[test]
    fn test_image_metadata() {
        let meta = ImageMetadata::new(MediaFormat::Png, 1920, 1080, 500_000, Encoding::Base64);
        assert_eq!(meta.max_dim(), 1920);
        assert!((meta.megapixels - 2.0736).abs() < 0.001);
    }
}
