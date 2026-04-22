//! Anthropic message format parser.
//!
//! Anthropic uses `image` content blocks:
//! ```json
//! {
//!   "type": "image",
//!   "source": {
//!     "type": "base64",
//!     "media_type": "image/png",
//!     "data": "iVBOR..."
//!   }
//! }
//! ```
//! Or URL-based:
//! ```json
//! {
//!   "type": "image",
//!   "source": {
//!     "type": "url",
//!     "url": "https://..."
//!   }
//! }
//! ```

use anyhow::{Context, Result};
use serde_json::Value;

use super::{ExtractedImage, ImageRef};
use crate::inspector::image::fetch_url_safe;
use crate::mode::SafetyLimits;

/// Extract all images from an Anthropic-format payload.
pub fn extract_images(payload: &Value) -> Result<Vec<ExtractedImage>> {
    extract_images_with_limits(payload, &SafetyLimits::default())
}

/// Extract images with explicit safety limits.
pub fn extract_images_with_limits(
    payload: &Value,
    limits: &SafetyLimits,
) -> Result<Vec<ExtractedImage>> {
    use base64::engine::general_purpose;
    use base64::Engine;

    let mut images = Vec::new();
    let mut global_index = 0;

    let messages = payload
        .get("messages")
        .and_then(|m| m.as_array())
        .context("payload missing 'messages' array")?;

    for (msg_idx, message) in messages.iter().enumerate() {
        let content = match message.get("content") {
            Some(Value::Array(arr)) => arr,
            _ => continue,
        };

        for (part_idx, part) in content.iter().enumerate() {
            let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if part_type != "image" {
                continue;
            }

            // Fix #8: Cap total images extracted
            if global_index >= limits.max_images_extract {
                break;
            }

            let source = part
                .get("source")
                .context("image block missing 'source' field")?;
            let source_type = source
                .get("type")
                .and_then(|t| t.as_str())
                .context("source missing 'type'")?;

            let (data, image_ref) = match source_type {
                "base64" => {
                    let media_type = source
                        .get("media_type")
                        .and_then(|m| m.as_str())
                        .unwrap_or("image/png")
                        .to_string();
                    let b64_data = source
                        .get("data")
                        .and_then(|d| d.as_str())
                        .context("base64 source missing 'data'")?;

                    // Fix #9: Check base64 size before decoding
                    if b64_data.len() > limits.max_base64_bytes {
                        anyhow::bail!(
                            "base64 image data too large: {} bytes exceeds limit",
                            b64_data.len()
                        );
                    }

                    // Fix #23: Try padded then unpadded
                    let bytes = general_purpose::STANDARD
                        .decode(b64_data)
                        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(b64_data))
                        .context("failed to decode base64 image data")?;

                    (
                        bytes,
                        ImageRef::Base64 {
                            media_type,
                            base64: b64_data.to_string(),
                        },
                    )
                }
                "url" => {
                    let url = source
                        .get("url")
                        .and_then(|u| u.as_str())
                        .context("url source missing 'url'")?;

                    // Fix #1, #3: Use safe URL fetcher
                    let bytes = fetch_url_safe(url, limits)?;
                    (bytes, ImageRef::Url(url.to_string()))
                }
                other => {
                    anyhow::bail!("unsupported Anthropic source type: {}", other);
                }
            };

            images.push(ExtractedImage {
                message_index: msg_idx,
                content_index: part_idx,
                data,
                original_ref: image_ref,
                global_index,
            });
            global_index += 1;
        }
    }

    Ok(images)
}

/// Reconstruct an Anthropic payload with transformed image data.
///
/// Takes the original payload and a list of (global_index, new_data, new_mime) tuples.
/// Images with empty data are dropped from the payload.
pub fn reconstruct(payload: &Value, transformed: &[(usize, Vec<u8>, String)]) -> Result<Value> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;

    let mut result = payload.clone();
    let messages = result
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
        .context("payload missing 'messages' array")?;

    let mut global_index = 0;

    for message in messages.iter_mut() {
        let content = match message.get_mut("content") {
            Some(Value::Array(arr)) => arr,
            _ => continue,
        };

        let mut to_remove = Vec::new();

        for (part_idx, part) in content.iter_mut().enumerate() {
            let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if part_type != "image" {
                continue;
            }

            if let Some((_idx, new_data, new_mime)) =
                transformed.iter().find(|(idx, _, _)| *idx == global_index)
            {
                if new_data.is_empty() {
                    to_remove.push(part_idx);
                } else {
                    // Replace with new base64 data
                    let b64 = engine.encode(new_data);
                    if let Some(source) = part.get_mut("source") {
                        source["type"] = Value::String("base64".to_string());
                        source["media_type"] = Value::String(new_mime.clone());
                        source["data"] = Value::String(b64);
                        // Remove URL field if it was URL-based before
                        if let Some(obj) = source.as_object_mut() {
                            obj.remove("url");
                        }
                    }
                }
            }

            global_index += 1;
        }

        for idx in to_remove.into_iter().rev() {
            content.remove(idx);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_png_base64() -> String {
        use base64::Engine;
        let img = image::RgbaImage::new(100, 100);
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            100,
            100,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        base64::engine::general_purpose::STANDARD.encode(&buf)
    }

    #[test]
    fn test_extract_base64_image() {
        let b64 = make_png_base64();
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Describe this image"},
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": b64
                        }
                    }
                ]
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].message_index, 0);
        assert_eq!(images[0].content_index, 1);
        assert!(!images[0].data.is_empty());
    }

    #[test]
    fn test_extract_multiple_images() {
        let b64 = make_png_base64();
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {"type": "base64", "media_type": "image/png", "data": b64.clone()}
                    },
                    {"type": "text", "text": "Compare these two images"},
                    {
                        "type": "image",
                        "source": {"type": "base64", "media_type": "image/png", "data": b64}
                    }
                ]
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert_eq!(images.len(), 2);
    }

    #[test]
    fn test_extract_no_images() {
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Hello, no images"}
                ]
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn test_extract_string_content() {
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": "Just text, not an array"
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn test_reconstruct_replaces_image() {
        let b64 = make_png_base64();
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Describe this"},
                    {
                        "type": "image",
                        "source": {"type": "base64", "media_type": "image/png", "data": b64}
                    }
                ]
            }]
        });

        let new_data = vec![0x89, 0x50, 0x4E, 0x47];
        let transformed = vec![(0, new_data, "image/png".to_string())];

        let result = reconstruct(&payload, &transformed).unwrap();
        let source = &result["messages"][0]["content"][1]["source"];
        assert_eq!(source["type"], "base64");
        assert_eq!(source["media_type"], "image/png");
        assert!(!source["data"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_reconstruct_drops_image() {
        let b64 = make_png_base64();
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Describe this"},
                    {
                        "type": "image",
                        "source": {"type": "base64", "media_type": "image/png", "data": b64}
                    }
                ]
            }]
        });

        let transformed = vec![(0, Vec::new(), "image/png".to_string())];
        let result = reconstruct(&payload, &transformed).unwrap();
        let content = result["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }
}
