//! Document transformation (v2).
//!
//! Future capabilities:
//! - Chunk large documents for context window limits
//! - Summarize sections
//! - Extract text from PDF/DOCX
//! - Convert between formats (PDF -> text, HTML -> markdown)

use crate::policy::Action;
use anyhow::Result;

/// Apply a transformation action to document data.
///
/// **Not yet implemented.** Returns an error indicating document support is planned for v2.
pub fn transform_document(_data: &[u8], _action: &Action) -> Result<Vec<u8>> {
    anyhow::bail!("document transformation is not yet supported (planned for v2)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_transform_not_supported() {
        let result = transform_document(b"fake doc", &Action::Pass);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet supported"));
    }
}
