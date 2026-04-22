//! OpenAI chat completion message format parser.
//!
//! OpenAI uses `image_url` content parts:
//! ```json
//! {
//!   "type": "image_url",
//!   "image_url": {
//!     "url": "data:image/png;base64,iVBOR..." // or "https://..."
//!   }
//! }
//! ```

use anyhow::{Context, Result};
use serde_json::Value;

use super::{ExtractedImage, ImageRef};
use crate::inspector::decode_base64_image;

/// Extract all images from an OpenAI-format payload.
pub fn extract_images(payload: &Value) -> Result<Vec<ExtractedImage>> {
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
            if part_type != "image_url" {
                continue;
            }

            let url = part
                .get("image_url")
                .and_then(|iu| iu.get("url"))
                .and_then(|u| u.as_str())
                .context("image_url part missing url field")?;

            let (data, image_ref) = if url.starts_with("data:") {
                let (bytes, mime_hint) = decode_base64_image(url)?;
                let mime = mime_hint.unwrap_or_else(|| "image/png".to_string());
                // Extract the base64 portion
                let b64 = url.find(',').map(|pos| &url[pos + 1..]).unwrap_or(url);
                (
                    bytes,
                    ImageRef::DataUri {
                        mime_type: mime,
                        base64: b64.to_string(),
                    },
                )
            } else if url.starts_with("http://") || url.starts_with("https://") {
                let response = minreq::get(url)
                    .with_timeout(30)
                    .send()
                    .with_context(|| format!("failed to fetch image from {}", url))?;
                if response.status_code != 200 {
                    anyhow::bail!("failed to fetch {}: HTTP {}", url, response.status_code);
                }
                (response.as_bytes().to_vec(), ImageRef::Url(url.to_string()))
            } else {
                anyhow::bail!(
                    "unsupported image_url format: {}",
                    &url[..url.len().min(50)]
                );
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

/// Reconstruct an OpenAI payload with transformed image data.
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

        // Collect indices to remove (for dropped images)
        let mut to_remove = Vec::new();

        for (part_idx, part) in content.iter_mut().enumerate() {
            let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if part_type != "image_url" {
                continue;
            }

            // Find this image in the transformed list
            if let Some((_idx, new_data, new_mime)) =
                transformed.iter().find(|(idx, _, _)| *idx == global_index)
            {
                if new_data.is_empty() {
                    // Image was dropped
                    to_remove.push(part_idx);
                } else {
                    // Replace with new data
                    let b64 = engine.encode(new_data);
                    let data_uri = format!("data:{};base64,{}", new_mime, b64);
                    if let Some(image_url) = part.get_mut("image_url") {
                        image_url["url"] = Value::String(data_uri);
                    }
                }
            }

            global_index += 1;
        }

        // Remove dropped images (reverse order to preserve indices)
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

    fn make_png_data_uri() -> String {
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
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        format!("data:image/png;base64,{}", b64)
    }

    #[test]
    fn test_extract_single_image() {
        let data_uri = make_png_data_uri();
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What's in this image?"},
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].message_index, 0);
        assert_eq!(images[0].content_index, 1);
        assert_eq!(images[0].global_index, 0);
        assert!(!images[0].data.is_empty());
    }

    #[test]
    fn test_extract_multiple_images() {
        let data_uri = make_png_data_uri();
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": data_uri.clone()}},
                    {"type": "text", "text": "Compare these:"},
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].global_index, 0);
        assert_eq!(images[1].global_index, 1);
    }

    #[test]
    fn test_extract_no_images() {
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": "Hello, no images here"
            }]
        });

        let images = extract_images(&payload).unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn test_extract_across_messages() {
        let data_uri = make_png_data_uri();
        let payload = json!({
            "model": "gpt-4o",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "image_url", "image_url": {"url": data_uri.clone()}}
                    ]
                },
                {"role": "assistant", "content": "I see an image."},
                {
                    "role": "user",
                    "content": [
                        {"type": "image_url", "image_url": {"url": data_uri}}
                    ]
                }
            ]
        });

        let images = extract_images(&payload).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].message_index, 0);
        assert_eq!(images[1].message_index, 2);
    }

    #[test]
    fn test_reconstruct_replaces_image() {
        let data_uri = make_png_data_uri();
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

        // Simulate a transformed image (just some bytes)
        let new_data = vec![0x89, 0x50, 0x4E, 0x47]; // PNG header stub
        let transformed = vec![(0, new_data, "image/png".to_string())];

        let result = reconstruct(&payload, &transformed).unwrap();
        let url = result["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_reconstruct_drops_image() {
        let data_uri = make_png_data_uri();
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

        // Empty data means drop
        let transformed = vec![(0, Vec::new(), "image/png".to_string())];

        let result = reconstruct(&payload, &transformed).unwrap();
        let content = result["messages"][0]["content"].as_array().unwrap();
        // Should only have the text part left
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }
}
