//! Video transformation (v2).
//!
//! Future capabilities:
//! - Frame sampling (extract N frames at intervals)
//! - Keyframe extraction
//! - Downscale resolution
//! - Trim duration
//! - Convert to frame sequence (for models that accept images but not video)

use crate::policy::Action;
use anyhow::Result;

/// Apply a transformation action to video data.
///
/// **Not yet implemented.** Returns an error indicating video support is planned for v2.
pub fn transform_video(_data: &[u8], _action: &Action) -> Result<Vec<u8>> {
    anyhow::bail!("video transformation is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_transform_not_supported() {
        let result = transform_video(b"fake video", &Action::Pass);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
