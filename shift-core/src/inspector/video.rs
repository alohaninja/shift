//! Video inspection (v2).
//!
//! Future capabilities:
//! - Detect video format (MP4, WebM, MOV)
//! - Extract duration, resolution, frame rate, codec
//! - Identify keyframes
//! - Estimate token cost per frame

use super::ImageMetadata;
use anyhow::Result;

/// Inspect video bytes and extract metadata.
///
/// **Not yet implemented.** Returns an error indicating video support is planned for v2.
pub fn inspect_bytes(_data: &[u8]) -> Result<ImageMetadata> {
    anyhow::bail!("video inspection is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_not_supported() {
        let result = inspect_bytes(b"fake video data");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
