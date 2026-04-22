//! Audio inspection (v2).
//!
//! Future capabilities:
//! - Detect audio format (MP3, WAV, OGG, FLAC)
//! - Extract duration, sample rate, bitrate, channels
//! - Estimate transcription token cost

use super::ImageMetadata;
use anyhow::Result;

/// Inspect audio bytes and extract metadata.
///
/// **Not yet implemented.** Returns an error indicating audio support is planned for v2.
pub fn inspect_bytes(_data: &[u8]) -> Result<ImageMetadata> {
    anyhow::bail!("audio inspection is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_not_supported() {
        let result = inspect_bytes(b"fake audio data");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
