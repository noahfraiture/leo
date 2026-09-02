use std::{collections::HashMap, path::PathBuf};

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
    pub analysis_frame_sets_per_prompt: String,
    pub analysis_overlap_frame_sets: String,
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
    pub sample_every_secs: String,
}

/// Editable analysis-provider values.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiDraft {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

/// A form field that can receive a sanitized validation message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SettingsField {
    CameraName(u32),
    CameraRtspUrl(u32),
    CameraSampleEvery(u32),
    DataRoot,
    RecorderTimeout,
    AnalysisFrameSetsPerPrompt,
    AnalysisOverlapFrameSets,
    OpenAiBaseUrl,
}

/// Reports whether validation found an editable error for one camera.
pub fn camera_has_error(errors: &HashMap<SettingsField, String>, camera_id: u32) -> bool {
    [
        SettingsField::CameraName(camera_id),
        SettingsField::CameraRtspUrl(camera_id),
        SettingsField::CameraSampleEvery(camera_id),
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
            sample_every_secs: "1".into(),
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
            SettingsField::CameraSampleEvery(selected_id),
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
            .map(|camera| {
                let sample_every_ms = camera
                    .sample_every_secs
                    .parse::<u64>()
                    .ok()
                    .filter(|seconds| *seconds > 0)
                    .and_then(|seconds| seconds.checked_mul(1_000))
                    .unwrap_or_else(|| {
                        errors.insert(
                            SettingsField::CameraSampleEvery(camera.id),
                            "Enter a positive whole number of seconds.".into(),
                        );
                        0
                    });
                CameraSettings {
                    id: camera.id,
                    name: camera.name.clone(),
                    rtsp_url: camera.rtsp_url.clone(),
                    initially_included_in_analysis: camera.initially_included_in_analysis,
                    sample_every_ms,
                }
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
        let analysis_frame_sets_per_prompt = self
            .draft
            .analysis_frame_sets_per_prompt
            .parse::<u64>()
            .ok()
            .filter(|frame_sets| *frame_sets > 0)
            .unwrap_or_else(|| {
                errors.insert(
                    SettingsField::AnalysisFrameSetsPerPrompt,
                    "Enter a positive whole number within runtime limits.".into(),
                );
                0
            });
        let analysis_overlap_frame_sets = self
            .draft
            .analysis_overlap_frame_sets
            .parse::<u64>()
            .unwrap_or_else(|_| {
                errors.insert(
                    SettingsField::AnalysisOverlapFrameSets,
                    "Enter a nonnegative whole number.".into(),
                );
                0
            });
        let settings = Settings {
            schema_version: self.draft.schema_version,
            next_camera_id: self.draft.next_camera_id,
            cameras,
            data_root: self.draft.data_root.clone(),
            recorder_timeout_secs,
            analysis_frame_sets_per_prompt,
            analysis_overlap_frame_sets,
            openai: OpenAiSettings {
                api_key: self.draft.openai.api_key.clone(),
                model: self.draft.openai.model.clone(),
                base_url: (!self.draft.openai.base_url.trim().is_empty())
                    .then(|| self.draft.openai.base_url.clone()),
            },
            log_level: self.draft.log_level,
        };

        if let Err(validation_errors) = settings.validate() {
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
                    ValidationError::InvalidSamplingCadence { camera_id } => (
                        SettingsField::CameraSampleEvery(camera_id),
                        "Enter a positive whole number of seconds.",
                    ),
                    ValidationError::DataRootNotAbsolute { .. } => {
                        (SettingsField::DataRoot, "Choose an absolute folder path.")
                    }
                    ValidationError::InvalidRecorderTimeout => (
                        SettingsField::RecorderTimeout,
                        "Enter a positive timeout within runtime limits.",
                    ),
                    ValidationError::InvalidOpenAiBaseUrl => (
                        SettingsField::OpenAiBaseUrl,
                        "Enter an absolute HTTP or HTTPS URL.",
                    ),
                    ValidationError::InvalidAnalysisFrameSetsPerPrompt => (
                        SettingsField::AnalysisFrameSetsPerPrompt,
                        "Enter a positive whole number within runtime limits.",
                    ),
                    ValidationError::InvalidAnalysisOverlapFrameSets => (
                        SettingsField::AnalysisOverlapFrameSets,
                        "Enter a nonnegative whole number within runtime limits and smaller than frame sets per prompt.",
                    ),
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
                    sample_every_secs: (camera.sample_every_ms / 1_000).to_string(),
                })
                .collect(),
            data_root: settings.data_root,
            recorder_timeout_secs: settings.recorder_timeout_secs.to_string(),
            analysis_frame_sets_per_prompt: settings.analysis_frame_sets_per_prompt.to_string(),
            analysis_overlap_frame_sets: settings.analysis_overlap_frame_sets.to_string(),
            openai: OpenAiDraft {
                api_key: settings.openai.api_key,
                model: settings.openai.model,
                base_url: settings.openai.base_url.unwrap_or_default(),
            },
            log_level: settings.log_level,
        }
    }
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
