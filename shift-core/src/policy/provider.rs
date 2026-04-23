use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Constraints for a specific model or provider default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConstraints {
    pub max_images: usize,
    pub max_image_dim: u32,
    pub max_image_size_bytes: usize,
    #[serde(default)]
    pub max_image_megapixels: Option<f64>,
    pub supported_formats: Vec<String>,
}

/// Full provider profile with per-model constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub name: String,
    #[serde(default)]
    pub models: HashMap<String, ModelConstraints>,
    pub default: ModelConstraints,
}

impl ProviderProfile {
    /// Get constraints for a specific model, falling back to provider defaults.
    pub fn constraints_for(&self, model: Option<&str>) -> &ModelConstraints {
        if let Some(model_name) = model {
            if let Some(constraints) = self.models.get(model_name) {
                return constraints;
            }
        }
        &self.default
    }

    /// Load a provider profile from JSON bytes.
    pub fn from_json(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data).context("failed to parse provider profile JSON")
    }
}

// Embedded profiles compiled into the binary
const OPENAI_PROFILE: &str = include_str!("../../profiles/openai.json");
const ANTHROPIC_PROFILE: &str = include_str!("../../profiles/anthropic.json");

/// Load a built-in provider profile by name.
pub fn load_builtin(provider: &str) -> Result<ProviderProfile> {
    let json = match provider.to_lowercase().as_str() {
        "openai" => OPENAI_PROFILE,
        "anthropic" | "claude" => ANTHROPIC_PROFILE,
        _ => anyhow::bail!(
            "unknown provider '{}': supported providers are 'openai' and 'anthropic'",
            provider
        ),
    };
    serde_json::from_str(json).context("failed to parse built-in provider profile")
}

/// Load a provider profile from an external JSON file.
pub fn load_from_file(path: &str) -> Result<ProviderProfile> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read profile from {}", path))?;
    ProviderProfile::from_json(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_openai_profile() {
        let profile = load_builtin("openai").unwrap();
        assert_eq!(profile.name, "openai");
        assert_eq!(profile.default.max_images, 10);
        assert_eq!(profile.default.max_image_dim, 2048);
        assert!(profile.models.contains_key("gpt-4o"));
    }

    #[test]
    fn test_load_anthropic_profile() {
        let profile = load_builtin("anthropic").unwrap();
        assert_eq!(profile.name, "anthropic");
        assert_eq!(profile.default.max_images, 20);
        assert_eq!(profile.default.max_image_megapixels, Some(1.15));
    }

    #[test]
    fn test_load_claude_alias() {
        let profile = load_builtin("claude").unwrap();
        assert_eq!(profile.name, "anthropic");
    }

    #[test]
    fn test_unknown_provider() {
        assert!(load_builtin("unknown").is_err());
    }

    #[test]
    fn test_constraints_for_specific_model() {
        let profile = load_builtin("openai").unwrap();
        let constraints = profile.constraints_for(Some("gpt-4o"));
        assert_eq!(constraints.max_image_dim, 2048);
    }

    #[test]
    fn test_constraints_for_unknown_model_falls_back() {
        let profile = load_builtin("openai").unwrap();
        let constraints = profile.constraints_for(Some("gpt-99"));
        assert_eq!(constraints.max_image_dim, profile.default.max_image_dim);
    }

    #[test]
    fn test_constraints_for_none_uses_default() {
        let profile = load_builtin("openai").unwrap();
        let constraints = profile.constraints_for(None);
        assert_eq!(constraints.max_image_dim, profile.default.max_image_dim);
    }

    #[test]
    fn test_supported_formats() {
        let profile = load_builtin("openai").unwrap();
        assert!(profile
            .default
            .supported_formats
            .contains(&"png".to_string()));
        assert!(profile
            .default
            .supported_formats
            .contains(&"jpeg".to_string()));
        assert!(profile
            .default
            .supported_formats
            .contains(&"gif".to_string()));
        assert!(profile
            .default
            .supported_formats
            .contains(&"webp".to_string()));
    }
}
