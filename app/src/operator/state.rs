//! Route-independent operator state and its synchronous transitions.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use backend::{
    analysis::{AnalysisCheckpoint, AnalyzeSession, OpenAiConfig},
    profiles::{AnalysisProfile, MonitoringProfile},
    recording::{RecorderEvent, RecorderHandle, RecorderStatus, RecordingCamera},
    session::{OperatorAction, SessionCamera, SessionController, StoredSession, list_sessions},
};
use uuid::Uuid;

use super::Error;
use crate::settings::{CameraSettings, Settings};

/// One camera's operator-facing configuration, sampling participation, and recorder health.
pub struct CameraState {
    pub config: CameraSettings,
    pub participating: bool,
    pub active_monitoring_profile_id: u32,
    pub recorder_status: RecorderStatus,
}

/// Current lifecycle of the single recording session.
pub enum SessionRunState {
    Idle,
    Starting {
        directory: PathBuf,
    },
    Active {
        directory: PathBuf,
        controller: Option<SessionController>,
        started_at: Instant,
    },
    Stopping {
        directory: PathBuf,
    },
    Faulted {
        directory: PathBuf,
    },
}

/// One completed session plus its optional validated analysis checkpoint.
pub struct SessionListItem {
    pub stored: StoredSession,
    pub checkpoint: std::result::Result<Option<AnalysisCheckpoint>, String>,
}

/// Synchronous snapshot produced before the recorder Start await.
pub struct StartSessionRequest {
    pub directory: PathBuf,
    pub events_path: PathBuf,
    pub recording_cameras: Vec<RecordingCamera>,
    pub session_cameras: Vec<SessionCamera>,
    pub monitoring_profiles: Vec<MonitoringProfile>,
    pub metadata_error: Option<String>,
    pub recorder: RecorderHandle,
}

/// Active resources moved out of [`OperatorState`] before the recorder Stop await.
pub struct StopSessionRequest {
    pub directory: PathBuf,
    pub controller: Option<SessionController>,
    pub recorder: RecorderHandle,
}

/// Resources needed for one fatal recorder cleanup attempt.
pub struct FaultSessionRequest {
    pub directory: PathBuf,
    pub controller: Option<SessionController>,
    pub recorder: RecorderHandle,
    pub message: String,
}

/// Route-independent operator state for recording, completed sessions, and analysis.
pub struct OperatorState {
    pub cameras: Vec<CameraState>,
    pub selected_camera_id: Option<u32>,
    pub session: SessionRunState,
    pub sessions: Vec<SessionListItem>,
    /// Retained recordings that cannot yet enter analysis.
    pub incomplete_sessions: Vec<PathBuf>,
    pub selected_session_id: Option<Uuid>,
    pub running_analysis_id: Option<Uuid>,
    pub analysis_error: Option<(Uuid, String)>,
    pub model_config_error: Option<String>,
    pub message: Option<String>,
    /// Persistent warning independent of capture health and transient action messages.
    pub metadata_error: Option<String>,
    pub session_root: PathBuf,
    pub monitoring_profiles: Vec<MonitoringProfile>,
    pub analysis_profiles: Vec<AnalysisProfile>,
    pub selected_analysis_profile_id: u32,
    pub monitoring_config_error: Option<String>,
    pub openai: Option<OpenAiConfig>,
    recorder: RecorderHandle,
}

impl OperatorState {
    /// Builds initial camera state and discovers completed sessions from disk.
    pub fn new(
        settings: Settings,
        session_root: PathBuf,
        recorder: RecorderHandle,
    ) -> Result<Self, Error> {
        let monitoring_config_error = settings
            .validate_monitoring()
            .err()
            .map(|error| error.to_string());
        let model_config_error = settings
            .validate_analysis()
            .err()
            .map(|error| error.to_string());
        let openai = settings.openai_config();
        let cameras = settings.cameras;
        let selected_camera_id = cameras.first().map(|camera| camera.id);
        let cameras = cameras
            .into_iter()
            .map(|config| CameraState {
                participating: config.initially_included_in_analysis,
                active_monitoring_profile_id: config.initial_monitoring_profile_id,
                config,
                recorder_status: RecorderStatus::Stopped,
            })
            .collect();
        let mut operator = Self {
            cameras,
            selected_camera_id,
            session: SessionRunState::Idle,
            sessions: Vec::new(),
            incomplete_sessions: Vec::new(),
            selected_session_id: None,
            running_analysis_id: None,
            analysis_error: None,
            model_config_error,
            message: None,
            metadata_error: None,
            session_root,
            monitoring_profiles: settings.monitoring_profiles,
            analysis_profiles: settings.analysis_profiles,
            selected_analysis_profile_id: settings.default_analysis_profile_id,
            monitoring_config_error,
            openai,
            recorder,
        };
        if let Err(error) = operator.refresh_sessions() {
            operator.message = Some(format!(
                "Session list unavailable: {error}. Recording remains available."
            ));
        }
        Ok(operator)
    }

    /// Selects a configured camera and clears only non-fault action messages.
    pub fn select_camera(&mut self, camera_id: u32) -> Result<(), Error> {
        self.camera_index(camera_id)?;
        self.selected_camera_id = Some(camera_id);
        self.set_transient_message(None);
        Ok(())
    }

    /// Updates an action message without replacing the canonical recorder fault.
    pub fn set_transient_message(&mut self, message: Option<String>) {
        if !matches!(self.session, SessionRunState::Faulted { .. }) {
            self.message = message;
        }
    }

    /// Claims one completed-session analysis after validating all synchronous preconditions.
    pub fn begin_analysis(&mut self, checklist: String) -> Result<AnalyzeSession, Error> {
        let session_id = self
            .selected_session_id
            .ok_or(Error::AnalysisSessionNotSelected)?;
        let row_index = self
            .sessions
            .iter()
            .position(|row| row.stored.session.id == session_id)
            .ok_or(Error::AnalysisSessionNotSelected)?;
        let directory = self.sessions[row_index].stored.directory.clone();
        if !fs::symlink_metadata(directory.join("recording-complete"))
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
        {
            return Err(Error::AnalysisSessionIncomplete);
        }
        if !matches!(self.session, SessionRunState::Idle) {
            return Err(Error::AnalysisRequiresIdleSession);
        }
        if self.running_analysis_id.is_some() {
            return Err(Error::AnalysisRunning);
        }
        let persisted = self.sessions[row_index]
            .checkpoint
            .as_ref()
            .map_err(|_| Error::InvalidAnalysisCheckpoint)?
            .as_ref();
        let profile = match persisted {
            Some(checkpoint) => checkpoint.analysis_profile.clone(),
            None => {
                if self.model_config_error.is_some() {
                    return Err(Error::ModelConfigurationUnavailable);
                }
                self.analysis_profiles
                    .iter()
                    .find(|profile| profile.id == self.selected_analysis_profile_id)
                    .cloned()
                    .ok_or(Error::ModelConfigurationUnavailable)?
            }
        };
        let persisted_checklist = persisted.map(|checkpoint| checkpoint.checklist.clone());
        let openai = self
            .openai
            .clone()
            .ok_or(Error::ModelConfigurationUnavailable)?;
        let checklist = persisted_checklist.unwrap_or_else(|| checklist.trim().to_owned());
        if checklist.trim().is_empty() {
            return Err(Error::EmptyChecklist);
        }

        self.running_analysis_id = Some(session_id);
        self.analysis_error = None;
        Ok(AnalyzeSession {
            directory,
            checklist,
            profile,
            checkpoint_path: None,
            openai,
        })
    }

    /// Explicitly discards only an idle session's analysis checkpoint for a new profile selection.
    pub fn reset_analysis(&mut self, session_id: Uuid) -> Result<(), Error> {
        if !matches!(self.session, SessionRunState::Idle) || self.running_analysis_id.is_some() {
            return Err(Error::AnalysisRunning);
        }
        let row = self
            .sessions
            .iter_mut()
            .find(|row| row.stored.session.id == session_id)
            .ok_or(Error::AnalysisSessionNotSelected)?;
        let path = row.stored.directory.join("analysis.json");
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            return Err(Error::InvalidAnalysisCheckpoint);
        }
        fs::remove_file(path).map_err(Error::ResetAnalysis)?;
        row.checkpoint = Ok(None);
        self.analysis_error = None;
        Ok(())
    }

    /// Replaces one row with the backend's complete durable checkpoint snapshot.
    pub fn apply_checkpoint(&mut self, checkpoint: AnalysisCheckpoint) {
        let session_id = checkpoint.session_id;
        let complete = checkpoint.responses.len() == checkpoint.total_batches;
        if let Some(row) = self
            .sessions
            .iter_mut()
            .find(|row| row.stored.session.id == session_id)
        {
            row.checkpoint = Ok(Some(checkpoint));
        }
        if complete && self.running_analysis_id == Some(session_id) {
            self.running_analysis_id = None;
        }
    }

    /// Publishes a sanitized failure only for the currently running analysis.
    pub fn analysis_failed(&mut self, session_id: Uuid, message: String) {
        if self.running_analysis_id != Some(session_id) {
            return;
        }
        self.running_analysis_id = None;
        self.analysis_error = Some((session_id, message));
    }

    /// Creates exclusive staging storage and moves Idle to Starting.
    pub fn begin_start(&mut self, utc_ms: i64) -> Result<StartSessionRequest, Error> {
        if !matches!(self.session, SessionRunState::Idle) {
            return Err(Error::StartUnavailable);
        }
        if self.running_analysis_id.is_some() {
            return Err(Error::AnalysisRunning);
        }
        if self.cameras.is_empty() {
            return Err(Error::NoCamerasConfigured);
        }

        create_dir_all(&self.session_root)?;
        let directory = self.session_root.join(utc_ms.to_string());
        create_dir(&directory)?;
        let recordings_root = directory.join("recordings");
        create_dir(&recordings_root)?;
        for camera in &self.cameras {
            create_dir(&recordings_root.join(format!("camera-{}", camera.config.id)))?;
        }

        let camera_ids = self
            .cameras
            .iter()
            .map(|camera| camera.config.id)
            .collect::<Vec<_>>();
        let recording_cameras = self
            .cameras
            .iter()
            .map(|camera| RecordingCamera {
                id: camera.config.id,
                rtsp_url: camera.config.rtsp_url.clone(),
            })
            .collect();
        let session_cameras = self
            .cameras
            .iter()
            .map(|camera| SessionCamera {
                id: camera.config.id,
                name: camera.config.name.clone(),
                enabled: camera.participating,
                initial_monitoring_profile_id: camera.active_monitoring_profile_id,
            })
            .collect();
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Starting;
        }
        self.session = SessionRunState::Starting {
            directory: directory.clone(),
        };
        self.message = None;
        self.metadata_error = None;
        tracing::info!(
            path = %directory.display(),
            camera_ids = ?camera_ids,
            "session start requested"
        );

        Ok(StartSessionRequest {
            events_path: directory.join("events.jsonl"),
            directory,
            recording_cameras,
            session_cameras,
            monitoring_profiles: self.monitoring_profiles.clone(),
            metadata_error: self.monitoring_config_error.clone(),
            recorder: self.recorder.clone(),
        })
    }

    /// Publishes Active only if the matching Start transition is still current.
    pub fn finish_start(&mut self, directory: PathBuf, controller: Option<SessionController>) {
        if !matches!(
            &self.session,
            SessionRunState::Starting { directory: current } if current == &directory
        ) {
            return;
        }
        for camera in &mut self.cameras {
            if camera.recorder_status == RecorderStatus::Starting {
                camera.recorder_status = RecorderStatus::Recording;
            }
        }
        tracing::info!(path = %directory.display(), "session start completed");
        self.session = SessionRunState::Active {
            directory,
            controller,
            started_at: Instant::now(),
        };
        self.message = None;
    }

    /// Rolls a matching failed Start back to Idle with a visible error.
    pub fn fail_start(&mut self, directory: &Path, message: String) {
        if !matches!(
            &self.session,
            SessionRunState::Starting { directory: current }
                | SessionRunState::Stopping { directory: current }
                if current == directory
        ) {
            return;
        }
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Stopped;
        }
        tracing::error!(error = %message, "session start failed");
        self.session = SessionRunState::Idle;
        self.message = Some(message);
    }

    /// Moves the active controller out before finalization awaits begin.
    pub fn begin_stop(&mut self) -> Result<StopSessionRequest, Error> {
        let state = std::mem::replace(&mut self.session, SessionRunState::Idle);
        let SessionRunState::Active {
            directory,
            controller,
            ..
        } = state
        else {
            self.session = state;
            return Err(Error::StopUnavailable);
        };

        tracing::info!(path = %directory.display(), "session stop requested");
        self.session = SessionRunState::Stopping {
            directory: directory.clone(),
        };
        Ok(StopSessionRequest {
            directory,
            controller,
            recorder: self.recorder.clone(),
        })
    }

    /// Returns a finalized Stopping session to Idle and refreshes discovery.
    pub fn finish_stop(&mut self) -> Result<(), Error> {
        let SessionRunState::Stopping { directory } = &self.session else {
            return Err(Error::StopUnavailable);
        };
        tracing::info!(path = %directory.display(), "session stop completed");
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Stopped;
        }
        self.session = SessionRunState::Idle;
        self.message = None;
        self.refresh_sessions()
    }

    /// Claims one active/starting fatal cleanup and immediately blocks duplicates.
    pub fn begin_fault(
        &mut self,
        message: String,
        append_end: bool,
    ) -> Option<FaultSessionRequest> {
        let state = std::mem::replace(&mut self.session, SessionRunState::Idle);
        let (directory, controller) = match state {
            SessionRunState::Active {
                directory,
                controller,
                ..
            } => (directory, if append_end { controller } else { None }),
            SessionRunState::Starting { directory } => (directory, None),
            state => {
                self.session = state;
                return None;
            }
        };

        tracing::error!(path = %directory.display(), error = %message, "session fault cleanup requested");
        self.message = Some(message.clone());
        self.session = SessionRunState::Faulted {
            directory: directory.clone(),
        };
        Some(FaultSessionRequest {
            directory,
            controller,
            recorder: self.recorder.clone(),
            message,
        })
    }

    /// Records the final sanitized result of a matching cleanup attempt.
    pub fn finish_fault(&mut self, directory: PathBuf, message: String) {
        if !matches!(
            &self.session,
            SessionRunState::Starting { directory: current }
                | SessionRunState::Stopping { directory: current }
                | SessionRunState::Faulted { directory: current, .. }
                if current == &directory
        ) {
            return;
        }
        tracing::error!(path = %directory.display(), error = %message, "session fault cleanup finished");
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Stopped;
        }
        self.message = Some(message);
        self.session = SessionRunState::Faulted { directory };
    }

    /// Projects recorder status and fatal events into displayed camera health.
    pub fn apply_recorder_event(&mut self, event: &RecorderEvent) {
        if matches!(self.session, SessionRunState::Idle) {
            return;
        }
        match event {
            RecorderEvent::Status {
                camera_id, status, ..
            } => {
                if matches!(self.session, SessionRunState::Faulted { .. })
                    && *status != RecorderStatus::Stopped
                {
                    return;
                }
                if let Some(camera) = self
                    .cameras
                    .iter_mut()
                    .find(|camera| camera.config.id == *camera_id)
                {
                    camera.recorder_status = *status;
                }
            }
            RecorderEvent::Faulted {
                camera_id: Some(camera_id),
                ..
            } => {
                if let Some(camera) = self
                    .cameras
                    .iter_mut()
                    .find(|camera| camera.config.id == *camera_id)
                {
                    camera.recorder_status = RecorderStatus::Stopped;
                }
            }
            RecorderEvent::Faulted {
                camera_id: None, ..
            } => {
                for camera in &mut self.cameras {
                    camera.recorder_status = RecorderStatus::Stopped;
                }
            }
        }
    }

    /// Durably appends participation before changing its displayed value.
    pub fn set_participation(&mut self, camera_id: u32, enabled: bool) -> Result<(), Error> {
        let camera_index = self.camera_index(camera_id)?;
        if !matches!(self.session, SessionRunState::Idle) {
            self.record_metadata(OperatorAction::SetCameraParticipation { camera_id, enabled })?;
        }
        self.cameras[camera_index].participating = enabled;
        Ok(())
    }

    /// Selects one immutable monitoring definition for one or several cameras atomically.
    pub fn set_monitoring_profile(
        &mut self,
        camera_ids: Vec<u32>,
        monitoring_profile_id: u32,
    ) -> Result<(), Error> {
        if self.monitoring_config_error.is_some() {
            return Err(Error::MetadataUnavailable);
        }
        if camera_ids.is_empty() {
            return Err(backend::session::Error::EmptyCameraList.into());
        }
        let mut unique = std::collections::HashSet::new();
        for &camera_id in &camera_ids {
            self.camera_index(camera_id)?;
            if !unique.insert(camera_id) {
                return Err(backend::session::Error::DuplicateCamera { camera_id }.into());
            }
        }
        if !self
            .monitoring_profiles
            .iter()
            .any(|profile| profile.id == monitoring_profile_id)
        {
            return Err(backend::session::Error::from(
                backend::profiles::Error::UnknownMonitoring {
                    id: monitoring_profile_id,
                },
            )
            .into());
        }
        if !matches!(self.session, SessionRunState::Idle) {
            self.record_metadata(OperatorAction::SetMonitoringProfile {
                camera_ids: camera_ids.clone(),
                monitoring_profile_id,
            })?;
        }
        for camera in &mut self.cameras {
            if unique.contains(&camera.config.id) {
                camera.active_monitoring_profile_id = monitoring_profile_id;
            }
        }
        Ok(())
    }

    fn record_metadata(&mut self, action: OperatorAction) -> Result<(), Error> {
        let SessionRunState::Active { controller, .. } = &mut self.session else {
            return Err(Error::StopUnavailable);
        };
        let result = controller
            .as_mut()
            .ok_or(Error::MetadataUnavailable)?
            .apply(action);
        if let Err(error) = result {
            if error.is_write_failure() {
                // An uncertain append cannot be retried safely. Capture has independent ownership.
                *controller = None;
                self.metadata_error = Some(format!(
                    "Recording continues. Monitoring changes could not be saved: {error}. This session needs repair before analysis."
                ));
            }
            return Err(error.into());
        }
        Ok(())
    }

    /// Rebuilds marker-gated rows while preserving a still-present selection.
    pub fn refresh_sessions(&mut self) -> Result<(), Error> {
        let selected = self.selected_session_id;
        let catalogue = list_sessions(&self.session_root)?;
        self.incomplete_sessions = catalogue.incomplete;
        self.sessions = catalogue
            .sessions
            .into_iter()
            .map(|stored| {
                let checkpoint_path = stored.directory.join("analysis.json");
                let checkpoint = match fs::symlink_metadata(&checkpoint_path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    _ => AnalysisCheckpoint::read(&checkpoint_path, stored.session.id)
                        .map(Some)
                        .map_err(|error| error.to_string()),
                };
                SessionListItem { stored, checkpoint }
            })
            .collect();
        self.selected_session_id = selected
            .filter(|selected| {
                self.sessions
                    .iter()
                    .any(|row| row.stored.session.id == *selected)
            })
            .or_else(|| self.sessions.first().map(|row| row.stored.session.id));
        Ok(())
    }

    fn camera_index(&self, camera_id: u32) -> Result<usize, Error> {
        self.cameras
            .iter()
            .position(|camera| camera.config.id == camera_id)
            .ok_or(Error::UnknownCamera { camera_id })
    }
}

fn create_dir_all(path: &Path) -> Result<(), Error> {
    fs::create_dir_all(path).map_err(|source| Error::CreateDirectory {
        path: path.to_owned(),
        source,
    })
}

fn create_dir(path: &Path) -> Result<(), Error> {
    fs::create_dir(path).map_err(|source| Error::CreateDirectory {
        path: path.to_owned(),
        source,
    })
}

#[cfg(all(test, unix))]
#[path = "tests/state.rs"]
mod tests;
