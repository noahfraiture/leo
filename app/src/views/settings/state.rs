use std::{collections::HashMap, path::PathBuf};

use backend::profiles::MonitoringProfile;
use dioxus::prelude::Signal;

use crate::settings::{CameraSettings, LogLevel, Settings, SettingsStore, ValidationError};

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
    pub next_monitoring_profile_id: u32,
    pub workflow: crate::settings::WorkflowSettings,
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

/// A form field that can receive a sanitized validation message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SettingsField {
    CameraName(u32),
    CameraRtspUrl(u32),
    CameraMonitoringProfile(u32),
    DataRoot,
    RecorderTimeout,
    /// Stable model identity, or None for a shared execution/evidence policy field.
    WorkflowNumber {
        model_id: Option<String>,
        name: &'static str,
    },
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
    /// The last persisted snapshot, kept separate from edits made after a save begins.
    saved_draft: SettingsDraft,
    pub selected_camera_id: Option<u32>,
    pub field_errors: HashMap<SettingsField, String>,
    /// Preserve incomplete numeric edits across Settings navigation without changing saved values.
    pub number_inputs: HashMap<SettingsField, String>,
    pub save_error: Option<String>,
    /// A root-owned save is in progress, including when Settings is not the current route.
    pub saving: bool,
    pub restart_required: bool,
}

impl SettingsPageState {
    /// Creates an editable settings draft.
    pub fn new(settings: Settings) -> Self {
        let selected_camera_id = settings.cameras.first().map(|camera| camera.id);
        let draft = SettingsDraft::from(settings);
        Self {
            saved_draft: draft.clone(),
            draft,
            selected_camera_id,
            field_errors: HashMap::new(),
            number_inputs: HashMap::new(),
            save_error: None,
            saving: false,
            restart_required: false,
        }
    }

    /// Adds and selects the next monotonic camera ID, or reports exhaustion without editing.
    pub fn add_camera(&mut self) -> Option<u32> {
        let id = self.draft.next_camera_id;
        let Some(next_id) = id.checked_add(1) else {
            self.save_error =
                Some("Camera IDs are exhausted. Existing cameras remain editable.".into());
            return None;
        };
        self.draft.next_camera_id = next_id;
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
        Some(id)
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
        // Numeric controls retain invalid raw input separately from their last valid typed value.
        // Reusing only the typed value here would silently save a value the operator did not enter.
        let mut errors: HashMap<_, _> = self
            .field_errors
            .iter()
            .filter(|(field, _)| match field {
                SettingsField::WorkflowNumber { model_id: None, .. } => true,
                SettingsField::WorkflowNumber {
                    model_id: Some(id), ..
                } => self
                    .draft
                    .workflow
                    .models
                    .iter()
                    .any(|model| &model.id == id),
                _ => false,
            })
            .map(|(field, error)| (field.clone(), error.clone()))
            .collect();
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
            next_monitoring_profile_id: self.draft.next_monitoring_profile_id,
            log_level: self.draft.log_level,
            workflow: self.draft.workflow.clone(),
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
                    ValidationError::Profile(_) | ValidationError::InvalidNextProfileId => continue,
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

    /// Track exactly the saved snapshot, preserving later edits and unfinished numeric input.
    pub fn finish_save(&mut self, submitted: SettingsDraft) {
        if self.draft == submitted {
            self.number_inputs
                .retain(|field, _| self.field_errors.contains_key(field));
        }
        self.saved_draft = submitted;
        self.save_error = None;
        self.saving = false;
        self.restart_required = true;
    }

    /// Invalid numeric text is an edit even when the last valid typed value has not changed.
    pub fn has_unsaved_changes(&self) -> bool {
        self.draft != self.saved_draft
            || self
                .number_inputs
                .keys()
                .any(|field| self.field_errors.contains_key(field))
    }

    /// Remove a model and its unfinished numeric edits without touching another model's errors.
    pub fn remove_workflow_model(&mut self, id: &str) {
        self.draft.workflow.models.retain(|model| model.id != id);
        let keep = |field: &SettingsField| {
            !matches!(field,
            SettingsField::WorkflowNumber { model_id: Some(model_id), .. } if model_id == id)
        };
        self.field_errors.retain(|field, _| keep(field));
        self.number_inputs.retain(|field, _| keep(field));
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
            next_monitoring_profile_id: settings.next_monitoring_profile_id,
            log_level: settings.log_level,
            workflow: settings.workflow,
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
