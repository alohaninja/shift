//! Document inspection (v2).
//!
//! Future capabilities:
//! - Detect document format (PDF, DOCX, TXT, HTML, Markdown)
//! - Extract page count, word count, structure
//! - Estimate chunking strategy and token cost

use super::ImageMetadata;
use anyhow::Result;

/// Inspect document bytes and extract metadata.
///
/// **Not yet implemented.** Returns an error indicating document support is planned for v2.
pub fn inspect_bytes(_data: &[u8]) -> Result<ImageMetadata> {
    anyhow::bail!("document inspection is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_not_supported() {
        let result = inspect_bytes(b"fake document data");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
