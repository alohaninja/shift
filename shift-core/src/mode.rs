use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Drive mode controls the aggressiveness of transformations.
///
/// - **Performance**: minimal transforms, only enforce hard provider limits
/// - **Balanced**: moderate optimization, remove obvious waste (default)
/// - **Economy**: aggressive optimization, minimize token usage
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriveMode {
    Performance,
    #[default]
    Balanced,
    Economy,
}

impl fmt::Display for DriveMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriveMode::Performance => write!(f, "performance"),
            DriveMode::Balanced => write!(f, "balanced"),
            DriveMode::Economy => write!(f, "economy"),
        }
    }
}

impl FromStr for DriveMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "performance" | "perf" => Ok(DriveMode::Performance),
            "balanced" | "bal" => Ok(DriveMode::Balanced),
            "economy" | "eco" => Ok(DriveMode::Economy),
            _ => Err(format!(
                "unknown drive mode '{}': expected performance, balanced, or economy",
                s
            )),
        }
    }
}

/// SVG handling strategy.
///
/// - **Raster**: rasterize SVG to PNG before sending (default, safest)
/// - **Source**: pass SVG XML as text content instead of image
/// - **Hybrid**: rasterize but also include SVG source as text
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SvgMode {
    #[default]
    Raster,
    Source,
    Hybrid,
}

impl fmt::Display for SvgMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvgMode::Raster => write!(f, "raster"),
            SvgMode::Source => write!(f, "source"),
            SvgMode::Hybrid => write!(f, "hybrid"),
        }
    }
}

impl FromStr for SvgMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "raster" => Ok(SvgMode::Raster),
            "source" | "src" => Ok(SvgMode::Source),
            "hybrid" => Ok(SvgMode::Hybrid),
            _ => Err(format!(
                "unknown svg mode '{}': expected raster, source, or hybrid",
                s
            )),
        }
    }
}

/// Configuration bundle for a single SHIFT processing run.
#[derive(Debug, Clone)]
pub struct ShiftConfig {
    pub mode: DriveMode,
    pub svg_mode: SvgMode,
    pub provider: String,
    pub model: Option<String>,
    pub dry_run: bool,
    pub verbose: bool,
}

impl Default for ShiftConfig {
    fn default() -> Self {
        ShiftConfig {
            mode: DriveMode::default(),
            svg_mode: SvgMode::default(),
            provider: "openai".to_string(),
            model: None,
            dry_run: false,
            verbose: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drive_mode_from_str() {
        assert_eq!(
            "performance".parse::<DriveMode>().unwrap(),
            DriveMode::Performance
        );
        assert_eq!("perf".parse::<DriveMode>().unwrap(), DriveMode::Performance);
        assert_eq!(
            "balanced".parse::<DriveMode>().unwrap(),
            DriveMode::Balanced
        );
        assert_eq!("bal".parse::<DriveMode>().unwrap(), DriveMode::Balanced);
        assert_eq!("economy".parse::<DriveMode>().unwrap(), DriveMode::Economy);
        assert_eq!("eco".parse::<DriveMode>().unwrap(), DriveMode::Economy);
        assert!("invalid".parse::<DriveMode>().is_err());
    }

    #[test]
    fn test_drive_mode_display() {
        assert_eq!(DriveMode::Performance.to_string(), "performance");
        assert_eq!(DriveMode::Balanced.to_string(), "balanced");
        assert_eq!(DriveMode::Economy.to_string(), "economy");
    }

    #[test]
    fn test_svg_mode_from_str() {
        assert_eq!("raster".parse::<SvgMode>().unwrap(), SvgMode::Raster);
        assert_eq!("source".parse::<SvgMode>().unwrap(), SvgMode::Source);
        assert_eq!("src".parse::<SvgMode>().unwrap(), SvgMode::Source);
        assert_eq!("hybrid".parse::<SvgMode>().unwrap(), SvgMode::Hybrid);
        assert!("invalid".parse::<SvgMode>().is_err());
    }

    #[test]
    fn test_default_config() {
        let cfg = ShiftConfig::default();
        assert_eq!(cfg.mode, DriveMode::Balanced);
        assert_eq!(cfg.svg_mode, SvgMode::Raster);
        assert_eq!(cfg.provider, "openai");
        assert!(cfg.model.is_none());
        assert!(!cfg.dry_run);
        assert!(!cfg.verbose);
    }
}
