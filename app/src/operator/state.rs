//! Route-independent operator state and its synchronous transitions.

use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use backend::{
    analysis::{AnalysisCheckpoint, AnalyzeSession, OpenAiConfig},
    recording::{RecorderEvent, RecorderHandle, RecorderStatus, RecordingCamera},
    session::{OperatorAction, SessionCamera, SessionController, StoredSession, list_sessions},
};
use uuid::Uuid;

use super::Error;
use crate::settings::CameraSettings;

const MODEL_CONFIG_ERROR: &str = "Analysis requires an OpenAI API key and model in Settings.";

/// One camera's operator-facing configuration, sampling participation, and recorder health.
pub struct CameraState {
    pub config: CameraSettings,
    pub participating: bool,
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
        controller: SessionController,
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
    pub recorder: RecorderHandle,
}

/// Active resources moved out of [`OperatorState`] before the recorder Stop await.
pub struct StopSessionRequest {
    pub directory: PathBuf,
    pub controller: SessionController,
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
    pub selected_session_id: Option<Uuid>,
    pub running_analysis_id: Option<Uuid>,
    pub analysis_error: Option<(Uuid, String)>,
    pub model_config_error: Option<String>,
    pub message: Option<String>,
    pub session_root: PathBuf,
    /// Active startup frame-set count copied into each analysis request.
    pub analysis_frame_sets_per_prompt: NonZeroUsize,
    /// Active startup overlap copied into each analysis request.
    pub analysis_overlap_frame_sets: usize,
    openai: Option<OpenAiConfig>,
    recorder: RecorderHandle,
}

impl OperatorState {
    /// Builds initial camera state and discovers completed sessions from disk.
    pub fn new(
        cameras: Vec<CameraSettings>,
        session_root: PathBuf,
        recorder: RecorderHandle,
        openai: Option<OpenAiConfig>,
        analysis_frame_sets_per_prompt: NonZeroUsize,
        analysis_overlap_frame_sets: usize,
    ) -> Result<Self, Error> {
        let selected_camera_id = cameras.first().map(|camera| camera.id);
        let cameras = cameras
            .into_iter()
            .map(|config| CameraState {
                participating: config.initially_included_in_analysis,
                config,
                recorder_status: RecorderStatus::Stopped,
            })
            .collect();
        let mut operator = Self {
            cameras,
            selected_camera_id,
            session: SessionRunState::Idle,
            sessions: Vec::new(),
            selected_session_id: None,
            running_analysis_id: None,
            analysis_error: None,
            model_config_error: openai.is_none().then(|| MODEL_CONFIG_ERROR.to_owned()),
            message: None,
            session_root,
            analysis_frame_sets_per_prompt,
            analysis_overlap_frame_sets,
            openai,
            recorder,
        };
        operator.refresh_sessions()?;
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
    pub(crate) fn set_transient_message(&mut self, message: Option<String>) {
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
        let persisted_checklist = self.sessions[row_index]
            .checkpoint
            .as_ref()
            .map_err(|_| Error::InvalidAnalysisCheckpoint)?
            .as_ref()
            .map(|checkpoint| checkpoint.checklist.clone());
        if self.model_config_error.is_some() {
            return Err(Error::ModelConfigurationUnavailable);
        }
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
            frame_sets_per_prompt: self.analysis_frame_sets_per_prompt,
            overlap_frame_sets: self.analysis_overlap_frame_sets,
            openai,
        })
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
                sample_every: Duration::from_millis(camera.config.sample_every_ms),
            })
            .collect();
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Starting;
        }
        self.session = SessionRunState::Starting {
            directory: directory.clone(),
        };
        self.message = None;
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
            recorder: self.recorder.clone(),
        })
    }

    /// Publishes Active only if the matching Start transition is still current.
    pub fn finish_start(&mut self, directory: PathBuf, controller: SessionController) {
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
        };
        self.message = None;
    }

    /// Claims cleanup for a still-current failed Start before recorder Stop is awaited.
    pub(crate) fn claim_failed_start_cleanup(&mut self, directory: &Path) -> bool {
        if !matches!(
            &self.session,
            SessionRunState::Starting { directory: current } if current == directory
        ) {
            return false;
        }
        self.session = SessionRunState::Stopping {
            directory: directory.to_owned(),
        };
        true
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
            } => (directory, append_end.then_some(controller)),
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
        let SessionRunState::Active {
            directory,
            controller,
        } = &mut self.session
        else {
            return Err(Error::StopUnavailable);
        };
        controller.apply(OperatorAction::SetCameraParticipation { camera_id, enabled })?;
        self.cameras[camera_index].participating = enabled;
        tracing::info!(
            path = %directory.display(),
            camera_id,
            enabled,
            "session camera participation written"
        );
        Ok(())
    }

    /// Durably appends cadence before changing its displayed value.
    pub fn set_sampling_interval(
        &mut self,
        camera_id: u32,
        sample_every: Duration,
    ) -> Result<(), Error> {
        let camera_index = self.camera_index(camera_id)?;
        let sample_every_ms = u64::try_from(sample_every.as_millis())
            .map_err(|_| Error::InvalidSamplingInterval { camera_id })?;
        let SessionRunState::Active {
            directory,
            controller,
        } = &mut self.session
        else {
            return Err(Error::StopUnavailable);
        };
        controller.apply(OperatorAction::SetSamplingInterval {
            camera_id,
            sample_every,
        })?;
        self.cameras[camera_index].config.sample_every_ms = sample_every_ms;
        tracing::info!(
            path = %directory.display(),
            camera_id,
            sample_every_ms,
            "session camera cadence written"
        );
        Ok(())
    }

    /// Rebuilds marker-gated rows while preserving a still-present selection.
    pub fn refresh_sessions(&mut self) -> Result<(), Error> {
        let selected = self.selected_session_id;
        self.sessions = list_sessions(&self.session_root)?
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
