pub mod anthropic;
pub mod openai;

use serde_json::Value;

/// An extracted image from a message payload.
#[derive(Debug, Clone)]
pub struct ExtractedImage {
    /// Index of the message containing this image
    pub message_index: usize,
    /// Index of the content part within the message
    pub content_index: usize,
    /// The raw image data (decoded from base64 or fetched from URL)
    pub data: Vec<u8>,
    /// The original base64 string or URL (for reconstruction)
    pub original_ref: ImageRef,
    /// Sequential image index across the whole payload
    pub global_index: usize,
}

/// Reference to how the image was originally specified.
#[derive(Debug, Clone)]
pub enum ImageRef {
    /// Base64 data URI: data:image/png;base64,...
    DataUri { mime_type: String, base64: String },
    /// Plain base64 string (Anthropic style)
    Base64 { media_type: String, base64: String },
    /// URL reference
    Url(String),
}

/// Detect which provider format a payload uses.
pub fn detect_provider(payload: &Value) -> Option<&'static str> {
    // Anthropic uses top-level "messages" with content blocks having "type": "image"
    // OpenAI uses "messages" with content parts having "type": "image_url"
    if let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                for part in content {
                    if let Some(t) = part.get("type").and_then(|t| t.as_str()) {
                        if t == "image_url" {
                            return Some("openai");
                        }
                        if t == "image" {
                            return Some("anthropic");
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_detect_openai() {
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": "data:image/png;base64,abc"}
                }]
            }]
        });
        assert_eq!(detect_provider(&payload), Some("openai"));
    }

    #[test]
    fn test_detect_anthropic() {
        let payload = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/png", "data": "abc"}
                }]
            }]
        });
        assert_eq!(detect_provider(&payload), Some("anthropic"));
    }

    #[test]
    fn test_detect_text_only() {
        let payload = json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": "Hello world"
            }]
        });
        assert_eq!(detect_provider(&payload), None);
    }
}
