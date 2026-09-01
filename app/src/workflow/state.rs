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

/// Active resources moved out of Workflow before the recorder Stop await.
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

/// Shared route-independent recording and completed-session state.
pub struct Workflow {
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

impl Workflow {
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
        let mut workflow = Self {
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
        workflow.refresh_sessions()?;
        Ok(workflow)
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
mod tests {
    use std::{
        fs,
        num::NonZeroUsize,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };

    use backend::{
        analysis::{
            AnalysisCheckpoint, AnalysisResponse, AnalysisWarning, ChecklistProgress, Observation,
            OpenAiConfig,
        },
        recording::{
            RecorderEvent, RecorderRuntime, RecorderSettings, RecorderStatus, spawn_for_test,
        },
        session::{OperatorAction, SessionController, mark_recording_complete},
    };
    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{Error, SessionRunState, Workflow};
    use crate::settings::CameraSettings;

    const START_UTC_MS: i64 = 1_786_552_800_000;

    struct Harness {
        _temporary: TempDir,
        runtime: Option<RecorderRuntime>,
        workflow: Workflow,
    }

    impl Harness {
        fn new() -> Self {
            Self::with(camera_settings(), Some(crate::test_openai_config()))
        }

        fn with(cameras: Vec<CameraSettings>, openai: Option<OpenAiConfig>) -> Self {
            Self::with_batching(cameras, openai, NonZeroUsize::new(5).unwrap(), 0)
        }

        fn with_batching(
            cameras: Vec<CameraSettings>,
            openai: Option<OpenAiConfig>,
            frame_sets_per_prompt: NonZeroUsize,
            overlap_frame_sets: usize,
        ) -> Self {
            let temporary = tempfile::tempdir().expect("temporary root should be created");
            let executable = temporary.path().join("successful-preflight");
            fs::write(&executable, "#!/bin/sh\nexit 0\n")
                .expect("fake preflight executable should be written");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
                .expect("fake preflight executable should be executable");
            let (runtime, recorder, _events) = spawn_for_test(
                RecorderSettings {
                    io_timeout: Duration::from_secs(1),
                    retry_delay: Duration::from_secs(1),
                    stop_timeout: Duration::from_secs(1),
                },
                executable.clone(),
                executable,
            )
            .expect("test recorder runtime should start");
            let workflow = Workflow::new(
                cameras,
                temporary.path().join("sessions"),
                recorder,
                openai,
                frame_sets_per_prompt,
                overlap_frame_sets,
            )
            .expect("workflow should initialize");

            Self {
                _temporary: temporary,
                runtime: Some(runtime),
                workflow,
            }
        }

        fn shutdown(mut self) {
            self.runtime
                .take()
                .expect("runtime should be retained")
                .shutdown()
                .expect("runtime should shut down");
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            if let Some(runtime) = self.runtime.take() {
                let _ = runtime.shutdown();
            }
        }
    }

    fn camera_settings() -> Vec<CameraSettings> {
        vec![
            CameraSettings {
                id: 1,
                name: "Salon 1".into(),
                rtsp_url: "rtsp://camera-one.example/live".into(),
                initially_included_in_analysis: true,
                sample_every_ms: 1_000,
            },
            CameraSettings {
                id: 2,
                name: "Salon 2".into(),
                rtsp_url: "rtsp://camera-two.example/live".into(),
                initially_included_in_analysis: false,
                sample_every_ms: 2_000,
            },
        ]
    }

    fn start_active(workflow: &mut Workflow) -> PathBuf {
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("idle workflow should begin starting");
        let controller =
            SessionController::create(request.events_path.clone(), request.session_cameras.clone())
                .expect("session controller should be created");
        let directory = request.directory.clone();
        workflow.finish_start(directory.clone(), controller);
        directory
    }

    fn write_session(
        root: &Path,
        name: &str,
        session_id: Uuid,
        start_utc_ms: i64,
        marked: bool,
    ) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir_all(&directory).expect("session directory should be created");
        let events = [
            json!({
                "schema_version": 1,
                "sequence": 0,
                "session_id": session_id,
                "utc_ms": start_utc_ms,
                "session_offset_ms": 0,
                "action": {
                    "type": "session_started",
                    "cameras": [{
                        "camera_id": 1,
                        "name": "Salon 1",
                        "enabled": true,
                        "sample_every_ms": 1_000
                    }]
                }
            }),
            json!({
                "schema_version": 1,
                "sequence": 1,
                "session_id": session_id,
                "utc_ms": start_utc_ms + 1_000,
                "session_offset_ms": 1_000,
                "action": { "type": "session_ended" }
            }),
        ]
        .into_iter()
        .map(|event| serde_json::to_string(&event).expect("event should serialize"))
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        fs::write(directory.join("events.jsonl"), events)
            .expect("session events should be written");
        if marked {
            mark_recording_complete(&directory).expect("session should be marked complete");
        }
        directory
    }

    fn response(
        timestamp: &str,
        description: &str,
        summary: &str,
        status: &str,
    ) -> AnalysisResponse {
        AnalysisResponse {
            observations: vec![Observation {
                timestamp: timestamp.into(),
                description: description.into(),
            }],
            sequence_summary: summary.into(),
            checklist_progress: vec![ChecklistProgress {
                item: "Complete the exercise".into(),
                status: status.into(),
                note: format!("Evidence at {timestamp}"),
            }],
        }
    }

    fn checkpoint(
        session_id: Uuid,
        checklist: &str,
        total_batches: usize,
        responses: Vec<AnalysisResponse>,
    ) -> AnalysisCheckpoint {
        AnalysisCheckpoint {
            schema_version: 2,
            session_id,
            checklist: checklist.into(),
            plan_fingerprint: "0123456789abcdef".into(),
            total_batches,
            warnings: vec![AnalysisWarning::RecordingGap {
                camera_id: 2,
                start_offset_ms: 1_000,
                end_offset_ms: 2_000,
            }],
            responses,
        }
    }

    fn prepare_analysis_session(
        harness: &mut Harness,
        session_id: Uuid,
        saved: Option<&AnalysisCheckpoint>,
    ) -> PathBuf {
        let directory = write_session(
            &harness.workflow.session_root,
            &format!("session-{session_id}"),
            session_id,
            START_UTC_MS - 10_000,
            true,
        );
        if let Some(saved) = saved {
            fs::write(
                directory.join("analysis.json"),
                serde_json::to_vec_pretty(saved).expect("v2 checkpoint should serialize"),
            )
            .expect("v2 checkpoint should be written");
        }
        harness
            .workflow
            .refresh_sessions()
            .expect("completed session should be discovered");
        harness.workflow.selected_session_id = Some(session_id);
        harness.workflow.model_config_error = None;
        directory
    }

    fn begin_analysis_error(workflow: &mut Workflow, checklist: &str) -> Error {
        let Err(error) = workflow.begin_analysis(checklist.into()) else {
            panic!("analysis transition should be rejected");
        };
        error
    }

    fn saved_checkpoint(workflow: &Workflow, session_id: Uuid) -> &AnalysisCheckpoint {
        workflow
            .sessions
            .iter()
            .find(|row| row.stored.session.id == session_id)
            .expect("completed session row should remain")
            .checkpoint
            .as_ref()
            .expect("checkpoint should remain valid")
            .as_ref()
            .expect("checkpoint snapshot should be present")
    }

    #[test]
    fn construction_initializes_camera_selection_and_idle_state() {
        let harness = Harness::new();
        let workflow = &harness.workflow;

        assert_eq!(workflow.cameras.len(), 2);
        assert_eq!(workflow.selected_camera_id, Some(1));
        assert!(matches!(workflow.session, SessionRunState::Idle));
        assert!(workflow.cameras[0].participating);
        assert!(!workflow.cameras[1].participating);
        assert!(
            workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        assert!(workflow.sessions.is_empty());
        assert_eq!(workflow.selected_session_id, None);
        assert_eq!(workflow.running_analysis_id, None);
        assert_eq!(workflow.analysis_error, None);
        assert_eq!(workflow.model_config_error, None);
        assert_eq!(workflow.message, None);

        harness.shutdown();
    }

    #[test]
    fn camera_selection_accepts_configured_ids_and_rejects_unknown_ids() {
        let mut harness = Harness::new();

        harness
            .workflow
            .select_camera(2)
            .expect("configured camera should be selectable");
        assert_eq!(harness.workflow.selected_camera_id, Some(2));

        assert!(harness.workflow.select_camera(99).is_err());
        assert_eq!(harness.workflow.selected_camera_id, Some(2));

        harness.shutdown();
    }

    #[test]
    fn camera_selection_clears_transient_message_but_preserves_fault_message() {
        let mut harness = Harness::new();
        harness.workflow.message = Some("old action error".into());

        harness
            .workflow
            .select_camera(2)
            .expect("configured camera should be selectable");
        assert_eq!(harness.workflow.message, None);

        start_active(&mut harness.workflow);
        harness
            .workflow
            .begin_fault("canonical recorder fault".into(), true)
            .expect("active fault should be claimed");
        harness
            .workflow
            .select_camera(1)
            .expect("camera selection should remain available while faulted");
        harness
            .workflow
            .set_transient_message(Some("unrelated refresh error".into()));
        assert_eq!(
            harness.workflow.message.as_deref(),
            Some("canonical recorder fault")
        );

        harness.shutdown();
    }

    #[test]
    fn begin_start_creates_storage_and_records_every_camera() {
        let mut harness = Harness::new();

        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("idle workflow should begin starting");

        assert!(matches!(
            &harness.workflow.session,
            SessionRunState::Starting { directory } if directory == &request.directory
        ));
        assert_eq!(
            request.directory,
            harness.workflow.session_root.join(START_UTC_MS.to_string())
        );
        assert_eq!(request.events_path, request.directory.join("events.jsonl"));
        assert!(request.directory.is_dir());
        assert!(request.directory.join("recordings").is_dir());
        assert!(request.directory.join("recordings/camera-1").is_dir());
        assert!(request.directory.join("recordings/camera-2").is_dir());
        assert_eq!(
            request
                .recording_cameras
                .iter()
                .map(|camera| camera.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(
            request
                .session_cameras
                .iter()
                .map(|camera| (camera.id, camera.enabled, camera.sample_every))
                .collect::<Vec<_>>(),
            [
                (1, true, Duration::from_secs(1)),
                (2, false, Duration::from_secs(2)),
            ]
        );
        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Starting)
        );

        harness.shutdown();
    }

    #[test]
    fn empty_camera_start_is_rejected_before_creating_storage() {
        let mut harness = Harness::with(Vec::new(), None);
        let root = harness.workflow.session_root.clone();

        assert!(matches!(
            harness.workflow.begin_start(123),
            Err(Error::NoCamerasConfigured)
        ));
        assert!(!root.exists());
        harness.shutdown();
    }

    #[test]
    fn finish_and_fail_start_transition_to_active_or_idle() {
        let mut active = Harness::new();
        let directory = start_active(&mut active.workflow);

        assert!(matches!(
            &active.workflow.session,
            SessionRunState::Active { directory: actual, .. } if actual == &directory
        ));
        assert!(
            active
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Recording)
        );
        active.shutdown();

        let mut failed = Harness::new();
        let request = failed
            .workflow
            .begin_start(START_UTC_MS)
            .expect("idle workflow should begin starting");
        failed
            .workflow
            .fail_start(&request.directory, "camera startup failed".into());

        assert!(matches!(failed.workflow.session, SessionRunState::Idle));
        assert_eq!(
            failed.workflow.message.as_deref(),
            Some("camera startup failed")
        );
        assert!(
            failed
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        failed.shutdown();
    }

    #[test]
    fn stop_transitions_active_to_stopping_then_idle_and_refreshes() {
        let mut harness = Harness::new();
        let directory = start_active(&mut harness.workflow);

        let mut request = harness
            .workflow
            .begin_stop()
            .expect("active workflow should begin stopping");
        assert!(matches!(
            &harness.workflow.session,
            SessionRunState::Stopping { directory: actual } if actual == &directory
        ));
        request
            .controller
            .apply(OperatorAction::EndSession)
            .expect("session end should be written");
        mark_recording_complete(&directory).expect("session should be marked complete");
        harness
            .workflow
            .finish_stop()
            .expect("stopped session should refresh");

        assert!(matches!(harness.workflow.session, SessionRunState::Idle));
        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        assert_eq!(harness.workflow.sessions.len(), 1);
        assert_eq!(
            harness.workflow.selected_session_id,
            Some(harness.workflow.sessions[0].stored.session.id)
        );

        harness.shutdown();
    }

    #[test]
    fn duplicate_start_and_stop_requests_are_rejected() {
        let mut harness = Harness::new();

        assert!(harness.workflow.begin_stop().is_err());
        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("first Start should succeed");
        assert!(harness.workflow.begin_start(START_UTC_MS + 1).is_err());
        assert!(harness.workflow.begin_stop().is_err());

        let controller = SessionController::create(request.events_path, request.session_cameras)
            .expect("controller should be created");
        harness.workflow.finish_start(request.directory, controller);
        harness
            .workflow
            .begin_stop()
            .expect("first Stop should succeed");
        assert!(harness.workflow.begin_stop().is_err());

        harness.shutdown();
    }

    #[test]
    fn start_is_rejected_while_analysis_runs_before_creating_storage() {
        let mut harness = Harness::new();
        harness.workflow.running_analysis_id = Some(Uuid::from_u128(7));

        assert!(harness.workflow.begin_start(START_UTC_MS).is_err());
        assert!(matches!(harness.workflow.session, SessionRunState::Idle));
        assert!(!harness.workflow.session_root.exists());

        harness.shutdown();
    }

    #[test]
    fn fault_transition_retains_end_controller_once_when_requested() {
        let mut harness = Harness::new();
        let directory = start_active(&mut harness.workflow);

        let request = harness
            .workflow
            .begin_fault("recorder failed".into(), true)
            .expect("first active fault should request cleanup");

        assert_eq!(request.directory, directory);
        assert!(request.controller.is_some());
        assert_eq!(request.message, "recorder failed");
        assert!(matches!(
            &harness.workflow.session,
            SessionRunState::Faulted { directory: actual } if actual == &directory
        ));
        assert_eq!(harness.workflow.message.as_deref(), Some("recorder failed"));
        assert!(
            harness
                .workflow
                .begin_fault("duplicate".into(), true)
                .is_none()
        );

        harness.shutdown();
    }

    #[test]
    fn fatal_event_while_starting_cannot_reactivate_the_session() {
        let mut harness = Harness::new();
        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");

        let cleanup = harness
            .workflow
            .begin_fault("recorder failed after readiness".into(), true)
            .expect("a starting recorder fault should request cleanup");
        assert!(cleanup.controller.is_none());
        let controller = SessionController::create(request.events_path, request.session_cameras)
            .expect("late session controller should be constructible");
        harness.workflow.finish_start(request.directory, controller);

        assert!(matches!(
            harness.workflow.session,
            SessionRunState::Faulted { .. }
        ));

        harness.shutdown();
    }

    #[test]
    fn uncertain_metadata_fault_drops_controller_and_preserves_directory() {
        let mut harness = Harness::new();
        let directory = start_active(&mut harness.workflow);

        let request = harness
            .workflow
            .begin_fault("event append uncertain".into(), false)
            .expect("metadata fault should request recorder cleanup");

        assert_eq!(request.directory, directory);
        assert!(request.controller.is_none());
        assert!(directory.exists());
        harness.workflow.finish_fault(
            directory.clone(),
            "event append uncertain; recorder cleanup failed".into(),
        );
        assert!(matches!(
            &harness.workflow.session,
            SessionRunState::Faulted { directory: actual } if actual == &directory
        ));
        assert_eq!(
            harness.workflow.message.as_deref(),
            Some("event append uncertain; recorder cleanup failed")
        );
        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );

        harness.shutdown();
    }

    #[test]
    fn recorder_status_updates_only_the_target_and_reconnecting_never_faults() {
        let mut harness = Harness::new();
        start_active(&mut harness.workflow);

        harness
            .workflow
            .apply_recorder_event(&RecorderEvent::Status {
                camera_id: 2,
                status: RecorderStatus::Reconnecting,
                message: Some("camera stream interrupted".into()),
            });

        assert_eq!(
            harness.workflow.cameras[0].recorder_status,
            RecorderStatus::Recording
        );
        assert_eq!(
            harness.workflow.cameras[1].recorder_status,
            RecorderStatus::Reconnecting
        );
        assert!(matches!(
            harness.workflow.session,
            SessionRunState::Active { .. }
        ));

        harness.shutdown();
    }

    #[test]
    fn named_fatal_recorder_event_stops_only_the_affected_camera_immediately() {
        let mut harness = Harness::new();
        start_active(&mut harness.workflow);

        harness
            .workflow
            .apply_recorder_event(&RecorderEvent::Faulted {
                camera_id: Some(2),
                message: "camera recorder failed".into(),
            });

        assert_eq!(
            harness.workflow.cameras[0].recorder_status,
            RecorderStatus::Recording
        );
        assert_eq!(
            harness.workflow.cameras[1].recorder_status,
            RecorderStatus::Stopped
        );
        harness.shutdown();
    }

    #[test]
    fn global_fatal_recorder_event_stops_all_camera_health_immediately() {
        let mut harness = Harness::new();
        start_active(&mut harness.workflow);

        harness
            .workflow
            .apply_recorder_event(&RecorderEvent::Faulted {
                camera_id: None,
                message: "recorder runtime failed".into(),
            });

        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        harness.shutdown();
    }

    #[test]
    fn reconnecting_before_finish_start_is_preserved() {
        let mut harness = Harness::new();
        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        harness
            .workflow
            .apply_recorder_event(&RecorderEvent::Status {
                camera_id: 2,
                status: RecorderStatus::Reconnecting,
                message: Some("camera stream interrupted".into()),
            });
        let controller = SessionController::create(request.events_path, request.session_cameras)
            .expect("session controller should start");

        harness.workflow.finish_start(request.directory, controller);

        assert_eq!(
            harness.workflow.cameras[0].recorder_status,
            RecorderStatus::Recording
        );
        assert_eq!(
            harness.workflow.cameras[1].recorder_status,
            RecorderStatus::Reconnecting
        );
        harness.shutdown();
    }

    #[test]
    fn queued_starting_and_recording_after_failed_start_are_ignored() {
        let mut harness = Harness::new();
        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        harness
            .workflow
            .fail_start(&request.directory, "camera startup failed".into());

        for (camera_id, status) in [
            (1, RecorderStatus::Starting),
            (2, RecorderStatus::Recording),
        ] {
            harness
                .workflow
                .apply_recorder_event(&RecorderEvent::Status {
                    camera_id,
                    status,
                    message: None,
                });
        }

        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        harness.shutdown();
    }

    #[test]
    fn stopped_cleanup_status_is_applied_while_stopping_or_faulted() {
        let mut stopping = Harness::new();
        start_active(&mut stopping.workflow);
        stopping
            .workflow
            .begin_stop()
            .expect("active session should begin stopping");
        stopping
            .workflow
            .apply_recorder_event(&RecorderEvent::Status {
                camera_id: 1,
                status: RecorderStatus::Stopped,
                message: None,
            });
        assert_eq!(
            stopping.workflow.cameras[0].recorder_status,
            RecorderStatus::Stopped
        );
        stopping.shutdown();

        let mut faulted = Harness::new();
        start_active(&mut faulted.workflow);
        faulted
            .workflow
            .begin_fault("recorder failed".into(), true)
            .expect("active fault should claim cleanup");
        faulted
            .workflow
            .apply_recorder_event(&RecorderEvent::Status {
                camera_id: 2,
                status: RecorderStatus::Stopped,
                message: None,
            });
        assert_eq!(
            faulted.workflow.cameras[1].recorder_status,
            RecorderStatus::Stopped
        );
        faulted.shutdown();
    }

    #[test]
    fn late_non_stopped_statuses_cannot_revive_finalized_fault_health() {
        let mut harness = Harness::new();
        start_active(&mut harness.workflow);
        let request = harness
            .workflow
            .begin_fault("recorder failed".into(), true)
            .expect("active fault should claim cleanup");
        harness
            .workflow
            .finish_fault(request.directory, "cleanup finished".into());

        for (camera_id, status) in [
            (1, RecorderStatus::Recording),
            (2, RecorderStatus::Reconnecting),
        ] {
            harness
                .workflow
                .apply_recorder_event(&RecorderEvent::Status {
                    camera_id,
                    status,
                    message: None,
                });
        }

        assert!(
            harness
                .workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == RecorderStatus::Stopped)
        );
        harness.shutdown();
    }

    #[test]
    fn participation_is_written_before_display_state_changes() {
        let mut harness = Harness::new();
        let request = harness
            .workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let controller = SessionController::create(
            request.events_path,
            vec![request.session_cameras[0].clone()],
        )
        .expect("mismatched controller should be created for the failure test");
        harness.workflow.finish_start(request.directory, controller);

        let error = harness
            .workflow
            .set_participation(2, true)
            .expect_err("controller must reject an unknown session camera");

        assert!(!harness.workflow.cameras[1].participating);
        let cleanup = harness
            .workflow
            .begin_fault(error.to_string(), false)
            .expect("write failure should produce cleanup");
        assert!(cleanup.controller.is_none());

        harness.shutdown();
    }

    #[test]
    fn cadence_is_written_before_display_state_changes() {
        let mut harness = Harness::new();
        start_active(&mut harness.workflow);

        let error = harness
            .workflow
            .set_sampling_interval(2, Duration::ZERO)
            .expect_err("controller must reject zero cadence");

        assert_eq!(harness.workflow.cameras[1].config.sample_every_ms, 2_000);
        let cleanup = harness
            .workflow
            .begin_fault(error.to_string(), false)
            .expect("write failure should produce cleanup");
        assert!(cleanup.controller.is_none());

        harness.shutdown();
    }

    #[test]
    fn refresh_sessions_is_completion_marker_gated() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(11);
        let directory = write_session(
            &harness.workflow.session_root,
            "unmarked",
            session_id,
            1_000,
            false,
        );

        harness
            .workflow
            .refresh_sessions()
            .expect("unmarked refresh should succeed");
        assert!(harness.workflow.sessions.is_empty());

        mark_recording_complete(&directory).expect("session should be marked complete");
        harness
            .workflow
            .refresh_sessions()
            .expect("marked refresh should succeed");
        assert_eq!(harness.workflow.sessions.len(), 1);
        assert_eq!(harness.workflow.sessions[0].stored.session.id, session_id);

        harness.shutdown();
    }

    #[test]
    fn refresh_sessions_projects_invalid_checkpoint_as_a_row_error() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(12);
        let directory = write_session(
            &harness.workflow.session_root,
            "invalid-checkpoint",
            session_id,
            1_000,
            true,
        );
        fs::write(directory.join("analysis.json"), b"not JSON")
            .expect("invalid checkpoint should be written");

        harness
            .workflow
            .refresh_sessions()
            .expect("invalid checkpoint should not fail catalogue refresh");

        assert_eq!(harness.workflow.sessions.len(), 1);
        assert!(harness.workflow.sessions[0].checkpoint.is_err());
        assert_eq!(harness.workflow.selected_session_id, Some(session_id));

        harness.shutdown();
    }

    #[test]
    fn refresh_sessions_preserves_an_older_selection() {
        let mut harness = Harness::new();
        let oldest = Uuid::from_u128(21);
        let middle = Uuid::from_u128(22);
        let newest = Uuid::from_u128(23);
        write_session(
            &harness.workflow.session_root,
            "oldest",
            oldest,
            1_000,
            true,
        );
        write_session(
            &harness.workflow.session_root,
            "middle",
            middle,
            2_000,
            true,
        );
        harness
            .workflow
            .refresh_sessions()
            .expect("initial refresh should succeed");
        harness.workflow.selected_session_id = Some(oldest);
        write_session(
            &harness.workflow.session_root,
            "newest",
            newest,
            3_000,
            true,
        );

        harness
            .workflow
            .refresh_sessions()
            .expect("second refresh should succeed");

        assert_eq!(harness.workflow.selected_session_id, Some(oldest));
        assert_eq!(
            harness
                .workflow
                .sessions
                .iter()
                .map(|row| row.stored.session.id)
                .collect::<Vec<_>>(),
            [newest, middle, oldest]
        );

        harness.shutdown();
    }

    #[test]
    fn empty_checklist_missing_model_active_session_and_second_job_are_rejected() {
        let session_id = Uuid::from_u128(31);

        let mut unavailable = Harness::new();
        let directory = prepare_analysis_session(&mut unavailable, session_id, None);
        unavailable.workflow.selected_session_id = None;
        assert!(matches!(
            begin_analysis_error(&mut unavailable.workflow, ""),
            Error::AnalysisSessionNotSelected
        ));
        assert_eq!(unavailable.workflow.running_analysis_id, None);

        unavailable.workflow.selected_session_id = Some(session_id);
        fs::remove_file(directory.join("recording-complete"))
            .expect("completion marker should be removable");
        assert!(matches!(
            begin_analysis_error(&mut unavailable.workflow, "Complete the exercise"),
            Error::AnalysisSessionIncomplete
        ));
        assert_eq!(unavailable.workflow.running_analysis_id, None);
        mark_recording_complete(&directory).expect("completion marker should be restored");

        unavailable.workflow.sessions[0].checkpoint = Err("invalid checkpoint".into());
        unavailable.workflow.model_config_error = Some("model unavailable".into());
        assert!(matches!(
            begin_analysis_error(&mut unavailable.workflow, ""),
            Error::InvalidAnalysisCheckpoint
        ));
        assert_eq!(unavailable.workflow.running_analysis_id, None);

        unavailable.workflow.sessions[0].checkpoint = Ok(None);
        assert!(matches!(
            begin_analysis_error(&mut unavailable.workflow, ""),
            Error::ModelConfigurationUnavailable
        ));
        assert_eq!(unavailable.workflow.running_analysis_id, None);

        unavailable.workflow.model_config_error = None;
        assert!(matches!(
            begin_analysis_error(&mut unavailable.workflow, "  \n"),
            Error::EmptyChecklist
        ));
        assert_eq!(unavailable.workflow.running_analysis_id, None);
        unavailable.shutdown();

        let mut active = Harness::new();
        prepare_analysis_session(&mut active, session_id, None);
        active.workflow.selected_session_id = None;
        start_active(&mut active.workflow);
        assert!(matches!(
            begin_analysis_error(&mut active.workflow, "Complete the exercise"),
            Error::AnalysisSessionNotSelected
        ));
        active.workflow.selected_session_id = Some(session_id);
        assert!(matches!(
            begin_analysis_error(&mut active.workflow, "Complete the exercise"),
            Error::AnalysisRequiresIdleSession
        ));
        assert_eq!(active.workflow.running_analysis_id, None);
        active.shutdown();

        let mut running = Harness::new();
        prepare_analysis_session(&mut running, session_id, None);
        running
            .workflow
            .begin_analysis("Complete the exercise".into())
            .expect("first analysis should begin");
        running.workflow.sessions[0].checkpoint = Err("invalid checkpoint".into());
        running.workflow.model_config_error = Some("model unavailable".into());
        assert!(matches!(
            begin_analysis_error(&mut running.workflow, ""),
            Error::AnalysisRunning
        ));
        assert_eq!(running.workflow.running_analysis_id, Some(session_id));
        running.shutdown();
    }

    #[test]
    fn existing_checkpoint_locks_its_checklist() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(32);
        let persisted = checkpoint(session_id, "Persisted checklist\n", 2, Vec::new());
        let directory = prepare_analysis_session(&mut harness, session_id, Some(&persisted));

        let request = harness
            .workflow
            .begin_analysis("Replacement checklist".into())
            .expect("valid persisted analysis should resume");

        assert_eq!(request.directory, directory);
        assert_eq!(request.checklist, "Persisted checklist\n");
        assert_eq!(saved_checkpoint(&harness.workflow, session_id), &persisted);
        assert_eq!(harness.workflow.running_analysis_id, Some(session_id));
        harness.shutdown();
    }

    #[test]
    fn analysis_request_owns_the_startup_provider_and_batching_configuration() {
        let config = OpenAiConfig {
            api_key: "active-key".into(),
            model: "active-model".into(),
            base_url: Some("http://127.0.0.1:9000/v1".into()),
        };
        let mut harness = Harness::with_batching(
            camera_settings(),
            Some(config.clone()),
            NonZeroUsize::new(7).unwrap(),
            2,
        );
        let session_id = Uuid::from_u128(38);
        prepare_analysis_session(&mut harness, session_id, None);

        let request = harness
            .workflow
            .begin_analysis("Complete the exercise".into())
            .expect("analysis should begin");

        assert!(request.openai == config);
        assert_eq!(request.frame_sets_per_prompt.get(), 7);
        assert_eq!(request.overlap_frame_sets, 2);
        harness.shutdown();
    }

    #[test]
    fn checkpoint_snapshots_replace_instead_of_append() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(33);
        prepare_analysis_session(&mut harness, session_id, None);
        harness
            .workflow
            .begin_analysis("Complete the exercise".into())
            .expect("analysis should begin");
        let first = checkpoint(
            session_id,
            "Complete the exercise",
            3,
            vec![response("00:00:01", "First", "First summary", "started")],
        );
        let replacement = checkpoint(
            session_id,
            "Complete the exercise",
            3,
            vec![
                response("00:00:01", "First", "First summary", "started"),
                response("00:00:02", "Second", "Second summary", "continuing"),
            ],
        );

        harness.workflow.apply_checkpoint(first);
        harness.workflow.apply_checkpoint(replacement.clone());

        assert_eq!(
            saved_checkpoint(&harness.workflow, session_id),
            &replacement
        );
        assert_eq!(harness.workflow.running_analysis_id, Some(session_id));
        harness.shutdown();
    }

    #[test]
    fn all_observations_and_latest_summary_checklist_are_projected() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(34);
        prepare_analysis_session(&mut harness, session_id, None);
        let snapshot = checkpoint(
            session_id,
            "Complete the exercise",
            2,
            vec![
                response("00:00:01", "First", "First summary", "started"),
                response("00:00:02", "Second", "Latest summary", "respected"),
            ],
        );

        harness.workflow.apply_checkpoint(snapshot);

        let saved = saved_checkpoint(&harness.workflow, session_id);
        let observations = saved
            .responses
            .iter()
            .flat_map(|response| &response.observations)
            .map(|observation| {
                (
                    observation.timestamp.as_str(),
                    observation.description.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observations,
            [("00:00:01", "First"), ("00:00:02", "Second")]
        );
        let latest = saved
            .responses
            .last()
            .expect("latest response should exist");
        assert_eq!(latest.sequence_summary, "Latest summary");
        assert_eq!(latest.checklist_progress[0].status, "respected");
        assert_eq!(latest.checklist_progress[0].note, "Evidence at 00:00:02");
        harness.shutdown();
    }

    #[test]
    fn final_snapshot_and_failure_clear_the_matching_running_id() {
        let mut harness = Harness::new();
        let first_id = Uuid::from_u128(35);
        let second_id = Uuid::from_u128(36);
        prepare_analysis_session(&mut harness, first_id, None);
        prepare_analysis_session(&mut harness, second_id, None);
        harness.workflow.selected_session_id = Some(first_id);
        harness
            .workflow
            .begin_analysis("Complete the exercise".into())
            .expect("first analysis should begin");

        harness.workflow.apply_checkpoint(checkpoint(
            second_id,
            "Complete the exercise",
            1,
            vec![response("00:00:01", "Other", "Other", "respected")],
        ));
        assert_eq!(harness.workflow.running_analysis_id, Some(first_id));
        harness.workflow.apply_checkpoint(checkpoint(
            first_id,
            "Complete the exercise",
            2,
            vec![response("00:00:01", "Partial", "Partial", "started")],
        ));
        assert_eq!(harness.workflow.running_analysis_id, Some(first_id));
        harness.workflow.apply_checkpoint(checkpoint(
            first_id,
            "Complete the exercise",
            2,
            vec![
                response("00:00:01", "Partial", "Partial", "started"),
                response("00:00:02", "Final", "Final", "respected"),
            ],
        ));
        assert_eq!(harness.workflow.running_analysis_id, None);

        harness.workflow.selected_session_id = Some(second_id);
        harness
            .workflow
            .begin_analysis("Ignored for persisted result".into())
            .expect("second analysis should begin");
        harness
            .workflow
            .analysis_failed(first_id, "stale failure".into());
        assert_eq!(harness.workflow.running_analysis_id, Some(second_id));
        assert_eq!(harness.workflow.analysis_error, None);
        harness
            .workflow
            .analysis_failed(second_id, "provider unavailable".into());
        assert_eq!(harness.workflow.running_analysis_id, None);
        assert_eq!(
            harness.workflow.analysis_error,
            Some((second_id, "provider unavailable".into()))
        );
        harness.shutdown();
    }

    #[test]
    fn retry_preserves_the_saved_checkpoint() {
        let mut harness = Harness::new();
        let session_id = Uuid::from_u128(37);
        let persisted = checkpoint(
            session_id,
            "Persisted retry checklist",
            2,
            vec![response(
                "00:00:01",
                "Saved observation",
                "Saved summary",
                "started",
            )],
        );
        let directory = prepare_analysis_session(&mut harness, session_id, Some(&persisted));

        let first = harness
            .workflow
            .begin_analysis("Replacement".into())
            .expect("resume should begin");
        harness
            .workflow
            .analysis_failed(session_id, "temporary provider failure".into());
        assert_eq!(saved_checkpoint(&harness.workflow, session_id), &persisted);
        assert_eq!(harness.workflow.running_analysis_id, None);

        let retry = harness
            .workflow
            .begin_analysis("Another replacement".into())
            .expect("retry should begin");
        assert_eq!(first.directory, directory);
        assert_eq!(retry.directory, directory);
        assert_eq!(first.checklist, "Persisted retry checklist");
        assert_eq!(retry.checklist, "Persisted retry checklist");
        assert_eq!(saved_checkpoint(&harness.workflow, session_id), &persisted);
        assert_eq!(harness.workflow.analysis_error, None);
        assert_eq!(harness.workflow.running_analysis_id, Some(session_id));
        harness.shutdown();
    }
}
