//! Route-independent operator state and its synchronous transitions.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use backend::{
    profiles::MonitoringProfile,
    recording::{RecorderEvent, RecorderHandle, RecorderStatus, RecordingCamera},
    session::{
        OperatorAction, SessionCamera, SessionCatalog, SessionController, StoredSession,
        list_sessions,
    },
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
        /// Monotonic timeline origin, retained even if metadata later becomes unavailable.
        started_at: Instant,
    },
    Stopping {
        directory: PathBuf,
    },
    Faulted {
        directory: PathBuf,
    },
}

/// One completed source session available for independently versioned analysis runs.
pub struct SessionListItem {
    pub stored: StoredSession,
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
    /// Catalogue root read by the finalization worker after media and marker handling.
    pub session_root: PathBuf,
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

/// Route-independent recording state and completed-session selection.
pub struct OperatorState {
    pub cameras: Vec<CameraState>,
    pub selected_camera_id: Option<u32>,
    pub session: SessionRunState,
    pub sessions: Vec<SessionListItem>,
    /// Retained recordings that cannot yet enter analysis.
    pub incomplete_sessions: Vec<PathBuf>,
    pub selected_session_id: Option<Uuid>,
    /// Current catalogue read; newer discovery invalidates its result before it can replace rows.
    pub session_refresh_id: Option<Uuid>,
    /// Most recent finished manual refresh, exposed as a DOM acknowledgement for desktop checks.
    pub completed_session_refresh_id: Option<Uuid>,
    /// Analysis activity token held to prevent simultaneous recording startup.
    pub running_analysis_id: Option<Uuid>,
    pub message: Option<String>,
    /// Persistent warning independent of capture health and transient action messages.
    pub metadata_error: Option<String>,
    /// A closed recorder runtime cannot accept another session until the app restarts.
    pub recorder_error: Option<String>,
    pub session_root: PathBuf,
    pub monitoring_profiles: Vec<MonitoringProfile>,
    pub monitoring_config_error: Option<String>,
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
            session_refresh_id: None,
            completed_session_refresh_id: None,
            running_analysis_id: None,
            message: None,
            metadata_error: None,
            recorder_error: None,
            session_root,
            monitoring_profiles: settings.monitoring_profiles,
            monitoring_config_error,
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

    /// Add a failure without replacing other action errors or the canonical recorder fault.
    pub fn report_transient_error(&mut self, error: String) {
        let message = match &self.message {
            Some(message) => format!("{message}; {error}"),
            None => error,
        };
        self.set_transient_message(Some(message));
    }

    /// Reserves the Start transition before filesystem or recorder work begins.
    pub fn begin_start(&mut self, utc_ms: i64) -> Result<StartSessionRequest, Error> {
        if !matches!(self.session, SessionRunState::Idle) {
            return Err(Error::StartUnavailable);
        }
        if self.recorder_error.is_some() {
            return Err(Error::RecorderUnavailable);
        }
        if self.running_analysis_id.is_some() {
            return Err(Error::AnalysisRunning);
        }
        if self.cameras.is_empty() {
            return Err(Error::NoCamerasConfigured);
        }

        let directory = self.session_root.join(utc_ms.to_string());

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
        let elapsed = controller
            .as_ref()
            .map(SessionController::elapsed)
            .unwrap_or_default();
        let started_at = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        self.session = SessionRunState::Active {
            directory,
            controller,
            started_at,
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
        // Finalization owns the next catalogue publication; older reads must not publish during it.
        self.session_refresh_id = None;
        Ok(StopSessionRequest {
            directory,
            session_root: self.session_root.clone(),
            controller,
            recorder: self.recorder.clone(),
        })
    }

    /// Returns finalized capture to Idle and applies discovery already completed by its worker.
    pub fn finish_stop(&mut self, catalogue: Result<SessionCatalog, String>) -> Result<(), Error> {
        let SessionRunState::Stopping { directory } = &self.session else {
            return Err(Error::StopUnavailable);
        };
        tracing::info!(path = %directory.display(), "session stop completed");
        for camera in &mut self.cameras {
            camera.recorder_status = RecorderStatus::Stopped;
        }
        self.session = SessionRunState::Idle;
        self.message = None;
        self.session_refresh_id = None;
        self.apply_catalogue_result(catalogue);
        Ok(())
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
        self.session_refresh_id = None;
        let catalogue = list_sessions(&self.session_root)?;
        self.apply_catalogue(catalogue);
        Ok(())
    }

    /// Claim one background refresh unless discovery is already owned by a read or finalization.
    pub fn begin_session_refresh(&mut self) -> Option<(Uuid, PathBuf)> {
        if self.session_refresh_id.is_some()
            || matches!(self.session, SessionRunState::Stopping { .. })
        {
            return None;
        }
        let id = Uuid::new_v4();
        self.session_refresh_id = Some(id);
        self.set_transient_message(None);
        Some((id, self.session_root.clone()))
    }

    /// Publish only the current read, preserving selection changes and newer discovery.
    pub fn finish_session_refresh(&mut self, id: Uuid, result: Result<SessionCatalog, String>) {
        if self.session_refresh_id != Some(id) {
            return;
        }
        self.session_refresh_id = None;
        self.completed_session_refresh_id = Some(id);
        self.apply_catalogue_result(result);
    }

    fn apply_catalogue_result(&mut self, result: Result<SessionCatalog, String>) {
        match result {
            Ok(catalogue) => {
                self.apply_catalogue(catalogue);
            }
            Err(error) => {
                self.report_transient_error(format!(
                    "Session list could not be refreshed: {error}"
                ));
            }
        }
    }

    fn apply_catalogue(&mut self, catalogue: SessionCatalog) {
        let selected = self.selected_session_id;
        self.incomplete_sessions = catalogue.incomplete;
        self.sessions = catalogue
            .sessions
            .into_iter()
            .map(|stored| SessionListItem { stored })
            .collect();
        self.selected_session_id = selected
            .filter(|selected| {
                self.sessions
                    .iter()
                    .any(|row| row.stored.session.id == *selected)
            })
            .or_else(|| self.sessions.first().map(|row| row.stored.session.id));
    }

    fn camera_index(&self, camera_id: u32) -> Result<usize, Error> {
        self.cameras
            .iter()
            .position(|camera| camera.config.id == camera_id)
            .ok_or(Error::UnknownCamera { camera_id })
    }
}

#[cfg(all(test, unix))]
#[path = "tests/state.rs"]
mod tests;
