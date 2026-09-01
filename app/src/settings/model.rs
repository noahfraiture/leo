use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use backend::analysis::OpenAiConfig;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ValidationError, ValidationErrors};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

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

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            next_camera_id: 1,
            cameras: Vec::new(),
            data_root: None,
            recorder_timeout_secs: 10,
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
    /// Adds an editable camera draft with a fresh monotonic ID.
    pub fn add_camera(&mut self) -> Result<u32, ValidationError> {
        if self.next_camera_id == 0 {
            return Err(ValidationError::InvalidNextCameraId);
        }
        let id = self.next_camera_id;
        let next_camera_id = id
            .checked_add(1)
            .ok_or(ValidationError::CameraIdExhausted)?;
        self.cameras.push(CameraSettings {
            id,
            name: format!("Camera {id}"),
            rtsp_url: String::new(),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        });
        self.next_camera_id = next_camera_id;
        Ok(id)
    }

    /// Removes the camera with `camera_id` without making its ID reusable.
    pub fn remove_camera(&mut self, camera_id: u32) -> bool {
        let previous_len = self.cameras.len();
        self.cameras.retain(|camera| camera.id != camera_id);
        self.cameras.len() != previous_len
    }

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
            } else if !Url::parse(&camera.rtsp_url).is_ok_and(|url| url.scheme() == "rtsp") {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::settings::ValidationError;

    #[test]
    fn defaults_are_an_unconfigured_valid_draft() {
        let settings = Settings::default();
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.next_camera_id, 1);
        assert!(settings.cameras.is_empty());
        assert_eq!(settings.log_level, LogLevel::Info);
        settings.validate().unwrap();
    }

    #[test]
    fn generated_camera_ids_remain_monotonic_after_removal() {
        let mut settings = Settings::default();
        assert_eq!(settings.add_camera().unwrap(), 1);
        assert!(settings.remove_camera(1));
        assert_eq!(settings.add_camera().unwrap(), 2);
        assert_eq!(settings.next_camera_id, 3);
    }

    #[test]
    fn strict_schema_rejects_unknown_fields() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Settings>(value).is_err());
    }

    #[test]
    fn zero_and_multiple_cameras_are_valid() {
        let mut settings = Settings::default();
        settings.validate().unwrap();
        for id in 1..=3 {
            settings.cameras.push(CameraSettings {
                id,
                name: format!("Camera {id}"),
                rtsp_url: format!("rtsp://camera-{id}/stream"),
                initially_included_in_analysis: true,
                sample_every_ms: 1_000,
            });
        }
        settings.next_camera_id = 4;
        settings.validate().unwrap();
    }

    #[test]
    fn validation_returns_all_field_errors() {
        let settings = Settings {
            schema_version: SETTINGS_SCHEMA_VERSION + 1,
            next_camera_id: 0,
            cameras: vec![
                CameraSettings {
                    id: 0,
                    name: " ".into(),
                    rtsp_url: "".into(),
                    initially_included_in_analysis: true,
                    sample_every_ms: 0,
                },
                CameraSettings {
                    id: 1,
                    name: "Camera 1".into(),
                    rtsp_url: "https://camera-1/stream".into(),
                    initially_included_in_analysis: false,
                    sample_every_ms: 1_500,
                },
                CameraSettings {
                    id: 1,
                    name: "Camera 1 duplicate".into(),
                    rtsp_url: "rtsp://camera-1/stream".into(),
                    initially_included_in_analysis: true,
                    sample_every_ms: 1_000,
                },
            ],
            data_root: Some(PathBuf::from("relative/data")),
            recorder_timeout_secs: 0,
            openai: OpenAiSettings {
                api_key: String::new(),
                model: String::new(),
                base_url: Some("ftp://provider.example/v1".into()),
            },
            log_level: LogLevel::Info,
        };

        let errors = settings.validate().unwrap_err();
        assert!(
            errors
                .0
                .contains(&ValidationError::UnsupportedSchemaVersion {
                    expected: SETTINGS_SCHEMA_VERSION,
                    actual: SETTINGS_SCHEMA_VERSION + 1,
                })
        );
        assert!(errors.0.contains(&ValidationError::InvalidNextCameraId));
        assert!(
            errors
                .0
                .contains(&ValidationError::ZeroCameraId { camera_index: 0 })
        );
        assert!(
            errors
                .0
                .contains(&ValidationError::DuplicateCameraId { camera_id: 1 })
        );
        assert!(
            errors
                .0
                .contains(&ValidationError::BlankCameraName { camera_id: 0 })
        );
        assert!(
            errors
                .0
                .contains(&ValidationError::BlankCameraUrl { camera_id: 0 })
        );
        assert!(
            errors
                .0
                .contains(&ValidationError::InvalidCameraUrl { camera_id: 1 })
        );
        assert!(
            errors
                .0
                .contains(&ValidationError::InvalidSamplingCadence { camera_id: 1 })
        );
        assert!(errors.0.contains(&ValidationError::InvalidRecorderTimeout));
        assert!(errors.0.contains(&ValidationError::DataRootNotAbsolute {
            path: PathBuf::from("relative/data"),
        }));
        assert!(errors.0.contains(&ValidationError::InvalidOpenAiBaseUrl));
    }

    #[test]
    fn camera_ids_must_be_below_the_next_allocation() {
        let mut settings = Settings::default();
        settings.next_camera_id = 1;
        settings.cameras.push(CameraSettings {
            id: 1,
            name: "Camera 1".into(),
            rtsp_url: "rtsp://camera-1/stream".into(),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        });

        assert!(settings.validate().unwrap_err().0.contains(
            &ValidationError::CameraIdNotBelowNext {
                camera_id: 1,
                next_camera_id: 1,
            }
        ));
    }

    #[test]
    fn camera_id_exhaustion_does_not_mutate_settings() {
        let mut settings = Settings::default();
        settings.next_camera_id = u32::MAX;

        assert_eq!(
            settings.add_camera(),
            Err(ValidationError::CameraIdExhausted)
        );
        assert!(settings.cameras.is_empty());
        assert_eq!(settings.next_camera_id, u32::MAX);
    }

    #[test]
    fn recorder_timeout_must_fit_runtime_limits() {
        let mut settings = Settings::default();
        settings.recorder_timeout_secs = u64::MAX;

        assert!(
            settings
                .validate()
                .unwrap_err()
                .0
                .contains(&ValidationError::InvalidRecorderTimeout)
        );
    }

    #[test]
    fn openai_config_requires_both_nonblank_values() {
        let mut settings = Settings::default();
        assert!(settings.openai_config().is_none());

        settings.openai.api_key = "key".into();
        settings.openai.model = " \t".into();
        assert!(settings.openai_config().is_none());

        settings.openai.model = "model".into();
        settings.openai.base_url = Some("https://provider.example/v1".into());
        let config = settings.openai_config().unwrap();
        assert_eq!(config.api_key, "key");
        assert_eq!(config.model, "model");
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://provider.example/v1")
        );
    }
}
