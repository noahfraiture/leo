//! Explicit sampling and model settings shared by recording metadata and analysis.

mod error;
mod model;

pub use error::Error;
pub use model::{
    AnalysisProfile, ImageDetailPolicy, ImageSizePolicy, MonitoringProfile,
    validate_analysis_profiles, validate_monitoring_profiles,
};

#[cfg(test)]
mod tests;
