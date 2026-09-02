use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use backend::analysis::OpenAiConfig;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ValidationError, ValidationErrors};

pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

/// Persisted application configuration edited before runtime services start.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    /// The next stable camera ID; removed IDs are never reused.
    pub next_camera_id: u32,
    pub cameras: Vec<CameraSettings>,
    pub data_root: Option<PathBuf>,
    pub recorder_timeout_secs: u64,
    /// Synchronized frame sets sent in each analysis request.
    pub analysis_frame_sets_per_prompt: u64,
    /// Frame sets repeated between adjacent analysis requests.
    pub analysis_overlap_frame_sets: u64,
    pub openai: OpenAiSettings,
    pub log_level: LogLevel,
}

/// One persisted camera source and its initial analysis behavior.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraSettings {
    pub id: u32,
    pub name: String,
    /// Credential-bearing source URL; do not include it in logs or errors.
    pub rtsp_url: String,
    pub initially_included_in_analysis: bool,
    /// Analysis sampling cadence in whole milliseconds.
    pub sample_every_ms: u64,
}

/// Persisted credentials and endpoint selection for explicit analysis.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenAiSettings {
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
}

/// Persisted application logging verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Returns the tracing filter directive for this persisted level.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            next_camera_id: 1,
            cameras: Vec::new(),
            data_root: None,
            recorder_timeout_secs: 10,
            analysis_frame_sets_per_prompt: 5,
            analysis_overlap_frame_sets: 0,
            openai: OpenAiSettings {
                api_key: String::new(),
                model: String::new(),
                base_url: None,
            },
            log_level: LogLevel::Info,
        }
    }
}

impl Settings {
    /// Returns every invalid persisted field or cross-field invariant.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();

        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            errors.push(ValidationError::UnsupportedSchemaVersion {
                expected: SETTINGS_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        if self.next_camera_id == 0 {
            errors.push(ValidationError::InvalidNextCameraId);
        }

        let mut camera_ids = HashSet::with_capacity(self.cameras.len());
        for (camera_index, camera) in self.cameras.iter().enumerate() {
            if camera.id == 0 {
                errors.push(ValidationError::ZeroCameraId { camera_index });
            }
            if !camera_ids.insert(camera.id) {
                errors.push(ValidationError::DuplicateCameraId {
                    camera_id: camera.id,
                });
            }
            if camera.id >= self.next_camera_id {
                errors.push(ValidationError::CameraIdNotBelowNext {
                    camera_id: camera.id,
                    next_camera_id: self.next_camera_id,
                });
            }
            if camera.name.trim().is_empty() {
                errors.push(ValidationError::BlankCameraName {
                    camera_id: camera.id,
                });
            }
            if camera.rtsp_url.trim().is_empty() {
                errors.push(ValidationError::BlankCameraUrl {
                    camera_id: camera.id,
                });
            } else if !valid_rtsp_url(&camera.rtsp_url) {
                errors.push(ValidationError::InvalidCameraUrl {
                    camera_id: camera.id,
                });
            }
            if camera.sample_every_ms == 0 || camera.sample_every_ms % 1_000 != 0 {
                errors.push(ValidationError::InvalidSamplingCadence {
                    camera_id: camera.id,
                });
            }
        }

        let recorder_timeout = Duration::from_secs(self.recorder_timeout_secs);
        if self.recorder_timeout_secs == 0
            || i64::try_from(recorder_timeout.as_micros()).is_err()
            || Instant::now().checked_add(recorder_timeout).is_none()
        {
            errors.push(ValidationError::InvalidRecorderTimeout);
        }
        if self.analysis_frame_sets_per_prompt == 0
            || usize::try_from(self.analysis_frame_sets_per_prompt).is_err()
        {
            errors.push(ValidationError::InvalidAnalysisFrameSetsPerPrompt);
        }
        if usize::try_from(self.analysis_overlap_frame_sets).is_err()
            || self.analysis_overlap_frame_sets >= self.analysis_frame_sets_per_prompt
        {
            errors.push(ValidationError::InvalidAnalysisOverlapFrameSets);
        }
        if let Some(path) = &self.data_root
            && !path.is_absolute()
        {
            errors.push(ValidationError::DataRootNotAbsolute { path: path.clone() });
        }
        if let Some(base_url) = &self.openai.base_url
            && !Url::parse(base_url)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
        {
            errors.push(ValidationError::InvalidOpenAiBaseUrl);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }

    /// Returns provider configuration only when both required values are present.
    pub fn openai_config(&self) -> Option<OpenAiConfig> {
        (!self.openai.api_key.trim().is_empty() && !self.openai.model.trim().is_empty()).then(
            || OpenAiConfig {
                api_key: self.openai.api_key.clone(),
                model: self.openai.model.clone(),
                base_url: self.openai.base_url.clone(),
            },
        )
    }
}

fn valid_rtsp_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url.scheme() == "rtsp" && url.has_host())
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;
