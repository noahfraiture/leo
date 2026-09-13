use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use backend::profiles::{MonitoringProfile, validate_monitoring_profiles};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ValidationError, ValidationErrors};

pub const SETTINGS_SCHEMA_VERSION: u32 = 4;

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
    pub monitoring_profiles: Vec<MonitoringProfile>,
    pub next_monitoring_profile_id: u32,
    pub workflow: super::WorkflowSettings,
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
    /// Optional metadata reference; invalid references do not affect capture.
    #[serde(default, deserialize_with = "profile_reference")]
    pub initial_monitoring_profile_id: u32,
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
            monitoring_profiles: vec![MonitoringProfile {
                id: 1,
                name: "Standard".into(),
                sample_every_ms: 1000,
            }],
            next_monitoring_profile_id: 2,
            log_level: LogLevel::Info,
            workflow: super::WorkflowSettings::default(),
        }
    }
}

impl Settings {
    /// Validates only the fields required to start and retain camera recordings.
    pub fn validate_recording(&self) -> Result<(), ValidationErrors> {
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
        }

        let recorder_timeout = Duration::from_secs(self.recorder_timeout_secs);
        if self.recorder_timeout_secs == 0
            || i64::try_from(recorder_timeout.as_micros()).is_err()
            || Instant::now().checked_add(recorder_timeout).is_none()
        {
            errors.push(ValidationError::InvalidRecorderTimeout);
        }
        if let Some(path) = &self.data_root
            && !path.is_absolute()
        {
            errors.push(ValidationError::DataRootNotAbsolute { path: path.clone() });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }

    /// Validates monitoring independently so invalid metadata cannot block recording.
    pub fn validate_monitoring(&self) -> Result<(), ValidationError> {
        validate_monitoring_profiles(&self.monitoring_profiles)
            .map_err(ValidationError::Profile)?;
        if self.next_monitoring_profile_id == 0
            || self
                .monitoring_profiles
                .iter()
                .any(|p| p.id >= self.next_monitoring_profile_id)
        {
            return Err(ValidationError::InvalidNextProfileId);
        }
        for camera in &self.cameras {
            if !self
                .monitoring_profiles
                .iter()
                .any(|p| p.id == camera.initial_monitoring_profile_id)
            {
                return Err(ValidationError::Profile(
                    backend::profiles::Error::UnknownMonitoring {
                        id: camera.initial_monitoring_profile_id,
                    },
                ));
            }
        }
        Ok(())
    }
}

// Invalid optional metadata references remain visible as unresolved IDs without rejecting capture settings.
fn profile_reference<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or(0))
}

fn valid_rtsp_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url.scheme() == "rtsp" && url.has_host())
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;
