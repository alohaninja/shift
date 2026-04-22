//! # SHIFT — Smart Hybrid Input Filtering & Transformation
//!
//! A multimodal preflight layer that automatically adapts inputs (images, video,
//! audio, documents, text) before they are sent to an AI model.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use shift_core::{pipeline, ShiftConfig, DriveMode};
//! use serde_json::json;
//!
//! let payload = json!({
//!     "model": "gpt-4o",
//!     "messages": [{"role": "user", "content": "Hello"}]
//! });
//!
//! let config = ShiftConfig {
//!     mode: DriveMode::Balanced,
//!     provider: "openai".to_string(),
//!     ..Default::default()
//! };
//!
//! let (transformed, report) = pipeline::process(&payload, &config).unwrap();
//! ```

pub mod cost;
pub mod inspector;
pub mod mode;
pub mod payload;
pub mod pipeline;
pub mod policy;
pub mod report;
pub mod transformer;

pub use cost::{ImageMetrics, TokenEstimate, TokenSavings};
pub use mode::{DriveMode, SafetyLimits, ShiftConfig, SvgMode};
pub use pipeline::process;
pub use report::Report;
