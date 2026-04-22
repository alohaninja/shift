//! Audio transformation (v2).
//!
//! Future capabilities:
//! - Compress / re-encode (lower bitrate)
//! - Transcribe to text (via external service)
//! - Trim silence
//! - Split into segments

use crate::policy::Action;
use anyhow::Result;

/// Apply a transformation action to audio data.
///
/// **Not yet implemented.** Returns an error indicating audio support is planned for v2.
pub fn transform_audio(_data: &[u8], _action: &Action) -> Result<Vec<u8>> {
    anyhow::bail!("audio transformation is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_transform_not_supported() {
        let result = transform_audio(b"fake audio", &Action::Pass);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
