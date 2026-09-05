use std::{collections::HashMap, path::PathBuf};

use backend::profiles::{AnalysisProfile, ImageDetailPolicy, ImageSizePolicy, MonitoringProfile};
use dioxus::prelude::Signal;

use crate::settings::{
    CameraSettings, LogLevel, OpenAiSettings, Settings, SettingsStore, ValidationError,
};

/// Root settings state and durable store shared by Settings components.
#[derive(Clone)]
pub struct SettingsContext {
    pub state: Signal<SettingsPageState>,
    pub store: SettingsStore,
}

/// Editable settings values, including numeric fields that may be temporarily invalid.
#[derive(Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    pub schema_version: u32,
    pub next_camera_id: u32,
    pub cameras: Vec<CameraDraft>,
    pub data_root: Option<PathBuf>,
    pub recorder_timeout_secs: String,
    pub monitoring_profiles: Vec<MonitoringProfileDraft>,
    pub analysis_profiles: Vec<AnalysisProfileDraft>,
    pub next_monitoring_profile_id: u32,
    pub next_analysis_profile_id: u32,
    pub default_analysis_profile_id: u32,
    pub openai: OpenAiDraft,
    pub log_level: LogLevel,
}

/// Editable values for one stable camera ID.
#[derive(Clone, PartialEq, Eq)]
pub struct CameraDraft {
    pub id: u32,
    pub name: String,
    pub rtsp_url: String,
    pub initially_included_in_analysis: bool,
    pub initial_monitoring_profile_id: u32,
}

/// Editable analysis-provider values.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiDraft {
    pub api_key: String,
    pub base_url: String,
}

/// A form field that can receive a sanitized validation message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SettingsField {
    CameraName(u32),
    CameraRtspUrl(u32),
    CameraMonitoringProfile(u32),
    DataRoot,
    RecorderTimeout,
}

/// Reports whether validation found an editable error for one camera.
pub fn camera_has_error(errors: &HashMap<SettingsField, String>, camera_id: u32) -> bool {
    [
        SettingsField::CameraName(camera_id),
        SettingsField::CameraRtspUrl(camera_id),
        SettingsField::CameraMonitoringProfile(camera_id),
    ]
    .iter()
    .any(|field| errors.contains_key(field))
}

/// Editable draft plus validation and save status.
#[derive(Clone)]
pub struct SettingsPageState {
    pub draft: SettingsDraft,
    pub selected_camera_id: Option<u32>,
    pub field_errors: HashMap<SettingsField, String>,
    pub save_error: Option<String>,
    pub restart_required: bool,
}

impl SettingsPageState {
    /// Creates an editable settings draft.
    pub fn new(settings: Settings) -> Self {
        let selected_camera_id = settings.cameras.first().map(|camera| camera.id);
        Self {
            draft: SettingsDraft::from(settings),
            selected_camera_id,
            field_errors: HashMap::new(),
            save_error: None,
            restart_required: false,
        }
    }

    /// Adds and selects a camera with the next monotonic ID.
    pub fn add_camera(&mut self) -> u32 {
        let id = self.draft.next_camera_id;
        self.draft.next_camera_id = id
            .checked_add(1)
            .expect("camera IDs should not be exhausted");
        self.draft.cameras.push(CameraDraft {
            id,
            name: format!("Camera {id}"),
            rtsp_url: String::new(),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: self
                .draft
                .monitoring_profiles
                .first()
                .map_or(0, |profile| profile.id),
        });
        self.selected_camera_id = Some(id);
        id
    }

    /// Removes only the selected camera and selects its nearest remaining neighbor.
    pub fn remove_selected_camera(&mut self) {
        let Some(selected_id) = self.selected_camera_id else {
            return;
        };
        let Some(index) = self
            .draft
            .cameras
            .iter()
            .position(|camera| camera.id == selected_id)
        else {
            self.selected_camera_id = self.draft.cameras.first().map(|camera| camera.id);
            return;
        };
        self.draft.cameras.remove(index);
        for field in [
            SettingsField::CameraName(selected_id),
            SettingsField::CameraRtspUrl(selected_id),
            SettingsField::CameraMonitoringProfile(selected_id),
        ] {
            self.field_errors.remove(&field);
        }
        self.selected_camera_id = self
            .draft
            .cameras
            .get(index)
            .or_else(|| self.draft.cameras.last())
            .map(|camera| camera.id);
    }

    /// Converts the current draft and returns every editable field error found.
    pub fn submission(&self) -> Result<Settings, HashMap<SettingsField, String>> {
        let mut errors = HashMap::new();
        let cameras = self
            .draft
            .cameras
            .iter()
            .map(|camera| CameraSettings {
                id: camera.id,
                name: camera.name.clone(),
                rtsp_url: camera.rtsp_url.clone(),
                initially_included_in_analysis: camera.initially_included_in_analysis,
                initial_monitoring_profile_id: camera.initial_monitoring_profile_id,
            })
            .collect();
        let recorder_timeout_secs = self
            .draft
            .recorder_timeout_secs
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds > 0)
            .unwrap_or_else(|| {
                errors.insert(
                    SettingsField::RecorderTimeout,
                    "Enter a positive whole number of seconds.".into(),
                );
                0
            });
        let settings = Settings {
            schema_version: self.draft.schema_version,
            next_camera_id: self.draft.next_camera_id,
            cameras,
            data_root: self.draft.data_root.clone(),
            recorder_timeout_secs,
            monitoring_profiles: self
                .draft
                .monitoring_profiles
                .iter()
                .map(MonitoringProfileDraft::profile)
                .collect(),
            analysis_profiles: self
                .draft
                .analysis_profiles
                .iter()
                .map(AnalysisProfileDraft::profile)
                .collect(),
            next_monitoring_profile_id: self.draft.next_monitoring_profile_id,
            next_analysis_profile_id: self.draft.next_analysis_profile_id,
            default_analysis_profile_id: self.draft.default_analysis_profile_id,
            openai: OpenAiSettings {
                api_key: self.draft.openai.api_key.clone(),
                base_url: (!self.draft.openai.base_url.trim().is_empty())
                    .then(|| self.draft.openai.base_url.clone()),
            },
            log_level: self.draft.log_level,
        };

        if let Err(validation_errors) = settings.validate_recording() {
            for error in validation_errors.0 {
                let (field, message) = match error {
                    ValidationError::BlankCameraName { camera_id } => {
                        (SettingsField::CameraName(camera_id), "Enter a camera name.")
                    }
                    ValidationError::BlankCameraUrl { camera_id }
                    | ValidationError::InvalidCameraUrl { camera_id } => (
                        SettingsField::CameraRtspUrl(camera_id),
                        "Enter a valid RTSP URL.",
                    ),
                    ValidationError::DataRootNotAbsolute { .. } => {
                        (SettingsField::DataRoot, "Choose an absolute folder path.")
                    }
                    ValidationError::InvalidRecorderTimeout => (
                        SettingsField::RecorderTimeout,
                        "Enter a positive timeout within runtime limits.",
                    ),
                    ValidationError::Profile(_)
                    | ValidationError::InvalidNextProfileId
                    | ValidationError::InvalidProvider => continue,
                    ValidationError::UnsupportedSchemaVersion { .. }
                    | ValidationError::InvalidNextCameraId
                    | ValidationError::ZeroCameraId { .. }
                    | ValidationError::DuplicateCameraId { .. }
                    | ValidationError::CameraIdNotBelowNext { .. } => {
                        panic!("settings draft invariant should remain valid")
                    }
                };
                errors.entry(field).or_insert_with(|| message.into());
            }
        }

        if errors.is_empty() {
            Ok(settings)
        } else {
            Err(errors)
        }
    }

    /// Marks the draft as saved; runtime services still require a restart.
    pub fn mark_saved(&mut self) {
        self.field_errors.clear();
        self.save_error = None;
        self.restart_required = true;
    }
}

impl From<Settings> for SettingsDraft {
    fn from(settings: Settings) -> Self {
        Self {
            schema_version: settings.schema_version,
            next_camera_id: settings.next_camera_id,
            cameras: settings
                .cameras
                .into_iter()
                .map(|camera| CameraDraft {
                    id: camera.id,
                    name: camera.name,
                    rtsp_url: camera.rtsp_url,
                    initially_included_in_analysis: camera.initially_included_in_analysis,
                    initial_monitoring_profile_id: camera.initial_monitoring_profile_id,
                })
                .collect(),
            data_root: settings.data_root,
            recorder_timeout_secs: settings.recorder_timeout_secs.to_string(),
            monitoring_profiles: settings
                .monitoring_profiles
                .into_iter()
                .map(MonitoringProfileDraft::from)
                .collect(),
            analysis_profiles: settings
                .analysis_profiles
                .into_iter()
                .map(AnalysisProfileDraft::from)
                .collect(),
            next_monitoring_profile_id: settings.next_monitoring_profile_id,
            next_analysis_profile_id: settings.next_analysis_profile_id,
            default_analysis_profile_id: settings.default_analysis_profile_id,
            openai: OpenAiDraft {
                api_key: settings.openai.api_key,
                base_url: settings.openai.base_url.unwrap_or_default(),
            },
            log_level: settings.log_level,
        }
    }
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;

/// Editable millisecond cadence, retaining invalid input for inline correction.
#[derive(Clone, PartialEq, Eq)]
pub struct MonitoringProfileDraft {
    pub id: u32,
    pub name: String,
    pub sample_every_ms: String,
}

impl From<MonitoringProfile> for MonitoringProfileDraft {
    fn from(profile: MonitoringProfile) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            sample_every_ms: profile.sample_every_ms.to_string(),
        }
    }
}
impl MonitoringProfileDraft {
    /// Invalid numeric input becomes an explicitly invalid profile, never a silent fallback cadence.
    pub fn profile(&self) -> MonitoringProfile {
        MonitoringProfile {
            id: self.id,
            name: self.name.clone(),
            sample_every_ms: self.sample_every_ms.parse().unwrap_or(0),
        }
    }
}

/// Editable model limits; invalid values disable analysis without blocking capture setup.
#[derive(Clone, PartialEq, Eq)]
pub struct AnalysisProfileDraft {
    pub id: u32,
    pub name: String,
    pub model: String,
    pub max_images: String,
    pub max_span_ms: String,
    pub overlap: String,
    pub maximum_edge: String,
    pub detail: ImageDetailPolicy,
    pub max_output_tokens: String,
}
impl From<AnalysisProfile> for AnalysisProfileDraft {
    fn from(profile: AnalysisProfile) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            model: profile.model,
            max_images: profile.max_images_per_prompt.to_string(),
            max_span_ms: profile.max_prompt_span_ms.to_string(),
            overlap: profile.overlap_frame_sets.to_string(),
            maximum_edge: match profile.image_size {
                ImageSizePolicy::Original => String::new(),
                ImageSizePolicy::MaximumLongEdge(edge) => edge.to_string(),
            },
            detail: profile.image_detail,
            max_output_tokens: profile
                .max_output_tokens
                .map(|value| value.to_string())
                .unwrap_or_default(),
        }
    }
}
impl AnalysisProfileDraft {
    /// Resolves the current editor values for validation and persistence.
    pub fn profile(&self) -> AnalysisProfile {
        AnalysisProfile {
            id: self.id,
            name: self.name.clone(),
            model: self.model.clone(),
            max_images_per_prompt: self.max_images.parse().unwrap_or(0),
            max_prompt_span_ms: self.max_span_ms.parse().unwrap_or(0),
            overlap_frame_sets: self.overlap.parse().unwrap_or(usize::MAX),
            image_size: if self.maximum_edge.trim().is_empty() {
                ImageSizePolicy::Original
            } else {
                ImageSizePolicy::MaximumLongEdge(self.maximum_edge.parse().unwrap_or(0))
            },
            image_detail: self.detail,
            max_output_tokens: (!self.max_output_tokens.trim().is_empty())
                .then(|| self.max_output_tokens.parse().unwrap_or(0)),
        }
    }
}
