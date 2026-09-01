use std::{collections::HashMap, path::PathBuf};

use dioxus::prelude::Signal;

use crate::settings::{
    CameraSettings, LogLevel, OpenAiSettings, ResolvedSettings, SaveOutcome, Settings,
    SettingsStore, ValidationError,
};

/// Root settings state and optional durable store shared by Settings components.
#[derive(Clone)]
pub struct SettingsContext {
    pub state: Signal<SettingsPageState>,
    pub store: Option<SettingsStore>,
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
    General,
}

/// Editable draft plus saved, active, and user-visible save status.
#[derive(Clone)]
pub struct SettingsPageState {
    pub draft: SettingsDraft,
    pub selected_camera_id: Option<u32>,
    pub saved: Option<ResolvedSettings>,
    pub active: Option<ResolvedSettings>,
    pub field_errors: HashMap<SettingsField, String>,
    pub load_error: Option<String>,
    pub save_error: Option<String>,
    pub saving: bool,
    pub restart_required: bool,
    pub durability_warning: Option<String>,
}

impl SettingsPageState {
    /// Creates an editable draft while retaining independent saved and active snapshots.
    pub fn new(
        settings: Settings,
        saved: Option<ResolvedSettings>,
        active: Option<ResolvedSettings>,
        load_error: Option<String>,
    ) -> Self {
        let selected_camera_id = settings.cameras.first().map(|camera| camera.id);
        Self {
            draft: SettingsDraft::from(settings),
            selected_camera_id,
            saved,
            active,
            field_errors: HashMap::new(),
            load_error,
            save_error: None,
            saving: false,
            restart_required: false,
            durability_warning: None,
        }
    }

    /// Adds and selects a camera with the next monotonic ID.
    pub fn add_camera(&mut self) -> Result<u32, ValidationError> {
        if self.draft.next_camera_id == 0 {
            return Err(ValidationError::InvalidNextCameraId);
        }
        let id = self.draft.next_camera_id;
        self.draft.next_camera_id = id
            .checked_add(1)
            .ok_or(ValidationError::CameraIdExhausted)?;
        self.draft.cameras.push(CameraDraft {
            id,
            name: format!("Camera {id}"),
            rtsp_url: String::new(),
            initially_included_in_analysis: true,
            sample_every_secs: "1".into(),
        });
        self.selected_camera_id = Some(id);
        Ok(id)
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
                    | ValidationError::CameraIdExhausted
                    | ValidationError::ZeroCameraId { .. }
                    | ValidationError::DuplicateCameraId { .. }
                    | ValidationError::CameraIdNotBelowNext { .. } => (
                        SettingsField::General,
                        "Settings state is inconsistent. Reload Leo and try again.",
                    ),
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

    /// Records a durable save while leaving active runtime settings untouched.
    pub fn apply_save(&mut self, outcome: SaveOutcome) {
        self.saved = Some(outcome.resolved);
        self.field_errors.clear();
        self.save_error = None;
        self.saving = false;
        self.restart_required = true;
        self.durability_warning = outcome.durability_warning;
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
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use backend::recording::RecorderSettings;

    use super::*;
    use crate::settings::{ResolvedSettings, SaveOutcome, Settings};

    fn resolved_settings(model: &str) -> ResolvedSettings {
        let root = std::env::temp_dir().join(format!("leo-settings-state-{model}"));
        let mut settings = Settings::default();
        settings.openai.api_key = "test-key".into();
        settings.openai.model = model.into();
        ResolvedSettings {
            openai: settings.openai_config(),
            log_level: settings.log_level,
            settings,
            settings_path: root.join("settings.json"),
            data_root: root.join("data"),
            sessions_root: root.join("data/sessions"),
            logs_root: root.join("data/logs"),
            recorder_settings: RecorderSettings {
                io_timeout: Duration::from_secs(10),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(5),
            },
            analysis_frame_sets_per_prompt: NonZeroUsize::new(5).unwrap(),
            analysis_overlap_frame_sets: 0,
        }
    }

    #[test]
    fn camera_ids_remain_monotonic_after_removing_the_selected_camera() {
        let mut state = SettingsPageState::new(Settings::default(), None, None, None);
        assert_eq!(state.add_camera().unwrap(), 1);
        state.remove_selected_camera();
        assert_eq!(state.add_camera().unwrap(), 2);
    }

    #[test]
    fn removing_a_camera_clears_only_its_field_errors() {
        let mut state = SettingsPageState::new(Settings::default(), None, None, None);
        state.add_camera().unwrap();
        let removed_id = state.add_camera().unwrap();
        for field in [
            SettingsField::CameraName(removed_id),
            SettingsField::CameraRtspUrl(removed_id),
            SettingsField::CameraSampleEvery(removed_id),
        ] {
            state.field_errors.insert(field, "invalid".into());
        }
        state
            .field_errors
            .insert(SettingsField::CameraName(1), "keep".into());

        state.remove_selected_camera();

        assert!(
            state
                .field_errors
                .contains_key(&SettingsField::CameraName(1))
        );
        assert!(!state.field_errors.keys().any(|field| matches!(
            field,
            SettingsField::CameraName(id)
                | SettingsField::CameraRtspUrl(id)
                | SettingsField::CameraSampleEvery(id)
                if *id == removed_id
        )));
    }

    #[test]
    fn successful_save_updates_saved_not_active_and_requires_restart() {
        let active = resolved_settings("active-model");
        let saved = resolved_settings("saved-model");
        let mut state = SettingsPageState::new(
            active.settings.clone(),
            Some(active.clone()),
            Some(active.clone()),
            None,
        );
        state.apply_save(SaveOutcome {
            resolved: saved.clone(),
            durability_warning: None,
        });

        assert_eq!(
            state.active.as_ref().unwrap().settings.openai.model,
            "active-model"
        );
        assert_eq!(
            state.saved.as_ref().unwrap().settings.openai.model,
            "saved-model"
        );
        assert!(state.restart_required);
    }

    #[test]
    fn draft_conversion_reports_camera_and_numeric_fields() {
        let mut state = SettingsPageState::new(Settings::default(), None, None, None);
        state.add_camera().unwrap();
        state.draft.cameras[0].rtsp_url = "http://wrong".into();
        state.draft.recorder_timeout_secs.clear();
        let errors = match state.submission() {
            Err(errors) => errors,
            Ok(_) => panic!("invalid draft should not produce settings"),
        };
        assert!(errors.contains_key(&SettingsField::CameraRtspUrl(1)));
        assert!(errors.contains_key(&SettingsField::RecorderTimeout));
    }

    #[test]
    fn draft_round_trips_analysis_batching_fields() {
        let settings = Settings {
            analysis_frame_sets_per_prompt: 7,
            analysis_overlap_frame_sets: 2,
            ..Settings::default()
        };
        let mut state = SettingsPageState::new(settings, None, None, None);

        assert_eq!(state.draft.analysis_frame_sets_per_prompt, "7");
        assert_eq!(state.draft.analysis_overlap_frame_sets, "2");

        state.draft.analysis_frame_sets_per_prompt = "9".into();
        state.draft.analysis_overlap_frame_sets = "3".into();
        let submitted = state.submission().expect("batching draft should be valid");
        assert_eq!(submitted.analysis_frame_sets_per_prompt, 9);
        assert_eq!(submitted.analysis_overlap_frame_sets, 3);
    }

    #[test]
    fn draft_maps_analysis_batching_errors_to_their_fields() {
        let mut state = SettingsPageState::new(Settings::default(), None, None, None);
        state.draft.analysis_frame_sets_per_prompt.clear();
        state.draft.analysis_overlap_frame_sets = "-1".into();

        let errors = match state.submission() {
            Err(errors) => errors,
            Ok(_) => panic!("invalid batching draft should fail"),
        };
        assert!(errors.contains_key(&SettingsField::AnalysisFrameSetsPerPrompt));
        assert!(errors.contains_key(&SettingsField::AnalysisOverlapFrameSets));

        state.draft.analysis_frame_sets_per_prompt = "5".into();
        state.draft.analysis_overlap_frame_sets = "5".into();
        let errors = match state.submission() {
            Err(errors) => errors,
            Ok(_) => panic!("overlapping batching draft should fail"),
        };
        assert_eq!(
            errors
                .get(&SettingsField::AnalysisOverlapFrameSets)
                .map(String::as_str),
            Some(
                "Enter a nonnegative whole number within runtime limits and smaller than frame sets per prompt."
            )
        );
    }
}
