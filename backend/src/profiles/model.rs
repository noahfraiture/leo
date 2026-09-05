use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::Error;

/// Sampling cadence selected in Monitor; capture always records continuously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonitoringProfile {
    pub id: u32,
    pub name: String,
    pub sample_every_ms: u64,
}

/// Model and evidence preparation settings fixed for one analysis run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisProfile {
    pub id: u32,
    pub name: String,
    pub model: String,
    pub max_images_per_prompt: usize,
    /// Maximum difference between the first and last frame-set timestamps in a request.
    pub max_prompt_span_ms: u64,
    /// Complete frame sets repeated from the preceding request, counted in its image limit.
    pub overlap_frame_sets: usize,
    pub image_size: ImageSizePolicy,
    pub image_detail: ImageDetailPolicy,
    pub max_output_tokens: Option<u64>,
}

/// Resizing applies to temporary evidence images only, preserving aspect ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageSizePolicy {
    Original,
    MaximumLongEdge(u32),
}

/// Provider image detail requested explicitly or left at its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageDetailPolicy {
    ProviderDefault,
    Low,
    High,
}

impl MonitoringProfile {
    /// Validates one complete profile independently from recording configuration.
    pub fn validate(&self) -> Result<(), Error> {
        validate_identities([(self.id, self.name.as_str())])?;
        if self.sample_every_ms == 0 {
            return Err(Error::Invalid {
                id: self.id,
                reason: "sampling interval must be positive milliseconds",
            });
        }
        Ok(())
    }
}

impl AnalysisProfile {
    /// Validates static limits; the batch planner additionally checks actual camera counts.
    pub fn validate(&self) -> Result<(), Error> {
        validate_identities([(self.id, self.name.as_str())])?;
        let reason = if self.model.trim().is_empty() {
            Some("enter a model name")
        } else if self.max_images_per_prompt == 0 || self.max_prompt_span_ms == 0 {
            Some("image count and maximum prompt span must be positive")
        } else if self.overlap_frame_sets >= self.max_images_per_prompt {
            Some("overlap must be smaller than the image limit")
        } else if self.max_output_tokens == Some(0) {
            Some("output token limit must be positive when set")
        } else if matches!(self.image_size, ImageSizePolicy::MaximumLongEdge(0)) {
            Some("maximum image edge must be positive")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(Error::Invalid {
                id: self.id,
                reason,
            }),
            None => Ok(()),
        }
    }
}

/// Validates definitions and uniqueness without consulting mutable application settings.
pub fn validate_monitoring_profiles(profiles: &[MonitoringProfile]) -> Result<(), Error> {
    validate_identities(
        profiles
            .iter()
            .map(|profile| (profile.id, profile.name.as_str())),
    )?;
    profiles.iter().try_for_each(MonitoringProfile::validate)
}

/// Validates definitions and uniqueness before assembling a provider request.
pub fn validate_analysis_profiles(profiles: &[AnalysisProfile]) -> Result<(), Error> {
    validate_identities(
        profiles
            .iter()
            .map(|profile| (profile.id, profile.name.as_str())),
    )?;
    profiles.iter().try_for_each(AnalysisProfile::validate)
}

fn validate_identities<'a>(
    profiles: impl IntoIterator<Item = (u32, &'a str)>,
) -> Result<(), Error> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for (id, name) in profiles {
        let reason = if id == 0 {
            Some("ID must be nonzero")
        } else if !ids.insert(id) {
            Some("ID is duplicated")
        } else if name.trim().is_empty() {
            Some("enter a profile name")
        } else if !names.insert(name.trim()) {
            Some("name is duplicated")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Error::Invalid { id, reason });
        }
    }
    Ok(())
}
