use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use backend::{
    recording::{Error as RecorderError, RecorderEvent, RecordingSegment},
    session::{OperatorAction, SessionController, mark_recording_complete},
};
use dioxus::prelude::{ReadableExt, Signal, WritableExt};

use crate::workflow::{
    FaultSessionRequest, SessionRunState, StartSessionRequest, StopSessionRequest, Workflow,
};

/// Starts one route-independent session task after claiming the synchronous transition.
pub fn spawn_start_session(mut workflow: Signal<Workflow>, utc_ms: i64) {
    let request = {
        let mut state = workflow.write();
        match state.begin_start(utc_ms) {
            Ok(request) => request,
            Err(error) => {
                state.set_transient_message(Some(error.to_string()));
                return;
            }
        }
    };
    let start_recorder = request.recorder.clone();
    let stop_recorder = request.recorder.clone();
    let recording_cameras = request.recording_cameras.clone();
    let recordings_root = request.directory.join("recordings");
    let current_workflow = workflow;
    let mut cleanup_workflow = workflow;

    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_start_session_with(
            request,
            async move {
                start_recorder
                    .start(recording_cameras, recordings_root)
                    .await
            },
            async move { stop_recorder.stop().await },
            move |directory| {
                let state = current_workflow.read();
                start_is_current(&state, directory)
            },
            move |directory| {
                cleanup_workflow
                    .write()
                    .claim_failed_start_cleanup(directory)
            },
        )
        .await;
        apply_start_outcome(&mut workflow.write(), outcome);
    });
}

/// Stops one active session without tying finalization to a route lifetime.
pub fn spawn_stop_session(mut workflow: Signal<Workflow>) {
    let request = {
        let mut state = workflow.write();
        match state.begin_stop() {
            Ok(request) => {
                state.set_transient_message(None);
                request
            }
            Err(error) => {
                state.set_transient_message(Some(error.to_string()));
                return;
            }
        }
    };
    let recorder = request.recorder.clone();

    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_stop_session_with(request, async move { recorder.stop().await }).await;
        apply_stop_outcome(&mut workflow.write(), outcome);
    });
}

/// Runs the single cleanup request already claimed by `Workflow::begin_fault`.
pub fn spawn_fault_cleanup(mut workflow: Signal<Workflow>, request: FaultSessionRequest) {
    let recorder = request.recorder.clone();
    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_fault_session_with(request, async move { recorder.stop().await }).await;
        apply_fault_outcome(&mut workflow.write(), outcome);
    });
}

/// Applies one recorder event and returns a newly claimed fatal cleanup, if any.
pub fn handle_recorder_event(
    workflow: &mut Workflow,
    event: RecorderEvent,
) -> Option<FaultSessionRequest> {
    workflow.apply_recorder_event(&event);
    match event {
        RecorderEvent::Status { .. } => None,
        RecorderEvent::Faulted { camera_id, message } => {
            let request = workflow.begin_fault(message, true);
            if request.is_some() {
                tracing::error!(camera_id, "fatal recorder event claimed by workflow");
            }
            request
        }
    }
}

/// Faults an in-flight session when its only recorder event boundary disappears.
pub fn handle_recorder_event_channel_closed(
    workflow: &mut Workflow,
) -> Option<FaultSessionRequest> {
    let message = "Recorder runtime stopped unexpectedly.".to_owned();
    let cleanup = workflow.begin_fault(message.clone(), true);
    if cleanup.is_none() {
        workflow.set_transient_message(Some(message));
    }
    cleanup
}

/// Persists participation before changing presentation state and faults on any write error.
pub fn set_participation(
    mut workflow: Signal<Workflow>,
    camera_id: u32,
    enabled: bool,
) -> Result<(), crate::workflow::Error> {
    let (result, cleanup) = {
        let mut state = workflow.write();
        match state.set_participation(camera_id, enabled) {
            Ok(()) => {
                state.set_transient_message(None);
                (Ok(()), None)
            }
            Err(error) => {
                let message = error.to_string();
                state.set_transient_message(Some(message.clone()));
                let cleanup = state.begin_fault(message, false);
                (Err(error), cleanup)
            }
        }
    };
    if let Some(request) = cleanup {
        spawn_fault_cleanup(workflow, request);
    }
    result
}

/// Persists cadence before changing presentation state and faults on any write error.
pub fn set_sampling_interval(
    mut workflow: Signal<Workflow>,
    camera_id: u32,
    sample_every: Duration,
) -> Result<(), crate::workflow::Error> {
    let (result, cleanup) = {
        let mut state = workflow.write();
        match state.set_sampling_interval(camera_id, sample_every) {
            Ok(()) => {
                state.set_transient_message(None);
                (Ok(()), None)
            }
            Err(error) => {
                let message = error.to_string();
                state.set_transient_message(Some(message.clone()));
                let cleanup = state.begin_fault(message, false);
                (Err(error), cleanup)
            }
        }
    };
    if let Some(request) = cleanup {
        spawn_fault_cleanup(workflow, request);
    }
    result
}

enum StartOutcome {
    Active {
        directory: PathBuf,
        controller: SessionController,
    },
    Idle {
        directory: PathBuf,
        message: String,
    },
    Faulted {
        directory: PathBuf,
        message: String,
    },
    Superseded,
}

enum StopOutcome {
    Completed,
    Faulted { directory: PathBuf, message: String },
}

struct FaultOutcome {
    directory: PathBuf,
    message: String,
}

async fn run_start_session_with<StartFuture, StopFuture, Continue, ClaimCleanup>(
    request: StartSessionRequest,
    start: StartFuture,
    stop: StopFuture,
    continue_start: Continue,
    claim_cleanup: ClaimCleanup,
) -> StartOutcome
where
    StartFuture: Future<Output = Result<(), RecorderError>>,
    StopFuture: Future<Output = Result<Vec<RecordingSegment>, RecorderError>>,
    Continue: FnOnce(&Path) -> bool,
    ClaimCleanup: FnOnce(&Path) -> bool,
{
    let camera_ids = request
        .recording_cameras
        .iter()
        .map(|camera| camera.id)
        .collect::<Vec<_>>();
    tracing::info!(
        path = %request.directory.display(),
        camera_ids = ?camera_ids,
        "recorder start awaiting all cameras"
    );
    if let Err(error) = start.await {
        let message = format!("Session start failed: {error}");
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "recorder start rolled back"
        );
        if matches!(error, RecorderError::RecorderStartupCleanupFailed) {
            return StartOutcome::Faulted {
                directory: request.directory,
                message,
            };
        }
        return remove_failed_start(request.directory, message);
    }
    if !continue_start(&request.directory) {
        tracing::info!(
            path = %request.directory.display(),
            "session start continuation owned by fault cleanup"
        );
        return StartOutcome::Superseded;
    }

    match SessionController::create(request.events_path, request.session_cameras) {
        Ok(controller) => StartOutcome::Active {
            directory: request.directory,
            controller,
        },
        Err(error) => {
            let metadata_message = format!("Session metadata start failed: {error}");
            if !claim_cleanup(&request.directory) {
                tracing::info!(
                    path = %request.directory.display(),
                    "failed session start cleanup owned by fault task"
                );
                return StartOutcome::Superseded;
            }
            tracing::error!(
                path = %request.directory.display(),
                error = %error,
                "session start event failed; recorder cleanup requested"
            );
            match stop.await {
                Ok(_) => remove_failed_start(request.directory, metadata_message),
                Err(cleanup_error) => StartOutcome::Faulted {
                    directory: request.directory,
                    message: format!(
                        "{metadata_message}; recorder cleanup failed: {cleanup_error}"
                    ),
                },
            }
        }
    }
}

fn remove_failed_start(directory: PathBuf, message: String) -> StartOutcome {
    match fs::remove_dir_all(&directory) {
        Ok(()) => StartOutcome::Idle { directory, message },
        Err(error) => StartOutcome::Idle {
            message: format!(
                "{message}; failed to remove staging directory {}: {error}",
                directory.display()
            ),
            directory,
        },
    }
}

fn apply_start_outcome(workflow: &mut Workflow, outcome: StartOutcome) {
    match outcome {
        StartOutcome::Active {
            directory,
            controller,
        } => workflow.finish_start(directory, controller),
        StartOutcome::Idle { directory, message } => workflow.fail_start(&directory, message),
        StartOutcome::Faulted { directory, message } => workflow.finish_fault(directory, message),
        StartOutcome::Superseded => {}
    }
}

fn start_is_current(workflow: &Workflow, directory: &Path) -> bool {
    matches!(
        &workflow.session,
        SessionRunState::Starting { directory: current } if current == directory
    )
}

async fn run_stop_session_with<StopFuture>(
    mut request: StopSessionRequest,
    stop: StopFuture,
) -> StopOutcome
where
    StopFuture: Future<Output = Result<Vec<RecordingSegment>, RecorderError>>,
{
    let end = request.controller.apply(OperatorAction::EndSession);
    if let Err(error) = &end {
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "session end event failed; recorder Stop still required"
        );
    } else {
        tracing::info!(path = %request.directory.display(), "session end event written");
    }

    let stopped = stop.await;
    if let Err(error) = &stopped {
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "recorder Stop failed"
        );
    } else {
        tracing::info!(path = %request.directory.display(), "recorder Stop completed");
    }

    let message = match (end, stopped) {
        (Ok(()), Ok(_)) => match mark_recording_complete(&request.directory) {
            Ok(()) => {
                tracing::info!(
                    path = %request.directory.display(),
                    "recording completion marker written"
                );
                return StopOutcome::Completed;
            }
            Err(error) => format!("Recording completion marker failed: {error}"),
        },
        (Err(error), Ok(_)) => format!("Session end event failed: {error}"),
        (Ok(()), Err(error)) => format!("Recorder Stop failed: {error}"),
        (Err(end_error), Err(stop_error)) => {
            format!("Session end event failed: {end_error}; recorder Stop failed: {stop_error}")
        }
    };
    StopOutcome::Faulted {
        directory: request.directory,
        message,
    }
}

fn apply_stop_outcome(workflow: &mut Workflow, outcome: StopOutcome) {
    match outcome {
        StopOutcome::Completed => {
            if let Err(error) = workflow.finish_stop() {
                workflow.set_transient_message(Some(format!(
                    "Completed session refresh failed: {error}"
                )));
            }
        }
        StopOutcome::Faulted { directory, message } => workflow.finish_fault(directory, message),
    }
}

async fn run_fault_session_with<StopFuture>(
    mut request: FaultSessionRequest,
    stop: StopFuture,
) -> FaultOutcome
where
    StopFuture: Future<Output = Result<Vec<RecordingSegment>, RecorderError>>,
{
    let end_error = request
        .controller
        .as_mut()
        .and_then(|controller| controller.apply(OperatorAction::EndSession).err());
    let stop_error = stop.await.err();
    let mut message = request.message;
    if let Some(error) = end_error {
        message.push_str(&format!("; session end event failed: {error}"));
    }
    if let Some(error) = stop_error {
        message.push_str(&format!("; recorder cleanup failed: {error}"));
    }
    tracing::error!(
        path = %request.directory.display(),
        append_end = request.controller.is_some(),
        cleanup_failed = message.contains("cleanup failed"),
        "fatal session cleanup finished"
    );
    FaultOutcome {
        directory: request.directory,
        message,
    }
}

fn apply_fault_outcome(workflow: &mut Workflow, outcome: FaultOutcome) {
    workflow.finish_fault(outcome.directory, outcome.message);
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use backend::{
        recording::{
            Error as RecorderError, RecorderEvent, RecorderHandle, RecorderRuntime,
            RecorderSettings, spawn_for_test,
        },
        session::{OperatorAction, Session},
    };
    use dioxus::prelude::Signal;
    use tokio::sync::oneshot;

    use super::{
        apply_fault_outcome, apply_start_outcome, apply_stop_outcome, handle_recorder_event,
        handle_recorder_event_channel_closed, run_fault_session_with, run_start_session_with,
        run_stop_session_with, set_participation, set_sampling_interval, spawn_fault_cleanup,
        spawn_start_session, spawn_stop_session, start_is_current,
    };
    use crate::{
        camera_config::CameraConfig,
        workflow::{Error, FaultSessionRequest, SessionRunState, Workflow},
    };

    const START_UTC_MS: i64 = 1_786_552_800_000;

    struct Harness {
        temporary: tempfile::TempDir,
        runtime: Option<RecorderRuntime>,
        recorder: RecorderHandle,
    }

    impl Harness {
        fn new() -> Self {
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
            Self {
                temporary,
                runtime: Some(runtime),
                recorder,
            }
        }

        fn session_root(&self) -> PathBuf {
            self.temporary.path().join("sessions")
        }

        fn workflow(&self) -> Workflow {
            Workflow::new(
                camera_configs(),
                self.session_root(),
                self.recorder.clone(),
                None,
            )
            .expect("workflow should initialize")
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

    fn camera_configs() -> Vec<CameraConfig> {
        vec![
            CameraConfig {
                id: 1,
                name: "Salon 1".into(),
                rtsp_url: "rtsp://camera-one.example/live".into(),
                enabled: true,
                sample_every_ms: 1_000,
            },
            CameraConfig {
                id: 2,
                name: "Salon 2".into(),
                rtsp_url: "rtsp://camera-two.example/live".into(),
                enabled: false,
                sample_every_ms: 2_000,
            },
        ]
    }

    #[test]
    fn root_session_tasks_keep_the_signal_orchestration_boundary() {
        let _: fn(Signal<Workflow>, i64) = spawn_start_session;
        let _: fn(Signal<Workflow>) = spawn_stop_session;
        let _: fn(Signal<Workflow>, FaultSessionRequest) = spawn_fault_cleanup;
        let _: fn(Signal<Workflow>, u32, bool) -> Result<(), Error> = set_participation;
        let _: fn(Signal<Workflow>, u32, Duration) -> Result<(), Error> = set_sampling_interval;
    }

    async fn make_active(workflow: &mut Workflow) -> PathBuf {
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            std::future::ready(Ok(Vec::new())),
            |_| true,
            |_| true,
        )
        .await;
        apply_start_outcome(workflow, outcome);
        assert!(matches!(workflow.session, SessionRunState::Active { .. }));
        directory
    }

    #[tokio::test]
    async fn start_actions_stop_reload_and_discovery_preserve_durable_order() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let events_path = request.events_path.clone();
        let directory = request.directory.clone();
        assert_eq!(
            request
                .recording_cameras
                .iter()
                .map(|camera| camera.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let start_events_path = events_path.clone();
        let start = async move {
            assert!(
                !start_events_path.exists(),
                "session start must follow all-camera readiness"
            );
            Ok(())
        };
        let failed_start_stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&failed_start_stop_polled);
        let unused_cleanup = async move {
            stop_flag.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        };

        let outcome =
            run_start_session_with(request, start, unused_cleanup, |_| true, |_| true).await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(!failed_start_stop_polled.load(Ordering::SeqCst));
        assert!(matches!(workflow.session, SessionRunState::Active { .. }));
        workflow
            .set_sampling_interval(2, Duration::from_secs(3))
            .expect("cadence should be durably updated");
        workflow
            .set_participation(2, true)
            .expect("participation should be durably updated");

        let stop_request = workflow
            .begin_stop()
            .expect("active session should begin stopping");
        let marker = directory.join("recording-complete");
        let stop_events_path = events_path.clone();
        let stop_marker = marker.clone();
        let stop = async move {
            Session::load(&stop_events_path)
                .expect("EndSession must be durable before recorder Stop is polled");
            assert!(
                !stop_marker.exists(),
                "completion marker must follow recorder Stop"
            );
            Ok(Vec::new())
        };
        let outcome = run_stop_session_with(stop_request, stop).await;
        apply_stop_outcome(&mut workflow, outcome);

        assert!(matches!(workflow.session, SessionRunState::Idle));
        assert!(marker.is_file());
        assert_eq!(fs::metadata(&marker).unwrap().len(), 0);
        let session = Session::load(&events_path).expect("completed events should reload");
        assert_eq!(
            session.actions,
            vec![
                (
                    session.actions[0].0,
                    OperatorAction::SetSamplingInterval {
                        camera_id: 2,
                        sample_every: Duration::from_secs(3),
                    },
                ),
                (
                    session.actions[1].0,
                    OperatorAction::SetCameraParticipation {
                        camera_id: 2,
                        enabled: true,
                    },
                ),
            ]
        );

        let reloaded = harness.workflow();
        assert_eq!(reloaded.sessions.len(), 1);
        assert_eq!(reloaded.sessions[0].stored.session.id, session.id);
        assert_eq!(reloaded.selected_session_id, Some(session.id));
        assert!(matches!(reloaded.sessions[0].checkpoint, Ok(None)));

        harness.shutdown();
    }

    #[tokio::test]
    async fn all_camera_startup_failure_rolls_back_staging_without_stop() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        let events_path = request.events_path.clone();
        let stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop_polled);

        let outcome = run_start_session_with(
            request,
            std::future::ready(Err(RecorderError::RecorderStartupFailed)),
            async move {
                stop_flag.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            },
            |_| true,
            |_| true,
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(matches!(workflow.session, SessionRunState::Idle));
        assert!(!directory.exists());
        assert!(!events_path.exists());
        assert!(!stop_polled.load(Ordering::SeqCst));
        assert!(workflow.message.is_some());

        harness.shutdown();
    }

    #[tokio::test]
    async fn uncertain_startup_cleanup_preserves_staging_and_faults() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();

        let outcome = run_start_session_with(
            request,
            std::future::ready(Err(RecorderError::RecorderStartupCleanupFailed)),
            std::future::ready(Ok(Vec::new())),
            |_| true,
            |_| true,
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert!(directory.is_dir());
        harness.shutdown();
    }

    #[tokio::test]
    async fn fault_claimed_before_start_continuation_creates_no_event_log() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let events_path = request.events_path.clone();
        let _cleanup = workflow
            .begin_fault("recorder failed after readiness".into(), true)
            .expect("fault should claim cleanup while Starting");

        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            std::future::ready(Ok(Vec::new())),
            |directory| start_is_current(&workflow, directory),
            |_| true,
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(!events_path.exists());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        harness.shutdown();
    }

    #[tokio::test]
    async fn recorder_event_channel_closure_faults_an_active_session_once() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        make_active(&mut workflow).await;

        let cleanup = handle_recorder_event_channel_closed(&mut workflow)
            .expect("channel closure should claim active cleanup");

        assert!(cleanup.controller.is_some());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert_eq!(
            workflow.message.as_deref(),
            Some("Recorder runtime stopped unexpectedly.")
        );
        assert!(handle_recorder_event_channel_closed(&mut workflow).is_none());
        assert_eq!(
            workflow.message.as_deref(),
            Some("Recorder runtime stopped unexpectedly.")
        );
        harness.shutdown();
    }

    #[tokio::test]
    async fn fault_owned_start_does_not_poll_second_stop_or_remove_directory() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        fs::write(&request.events_path, b"occupied")
            .expect("events path should force create_new failure");
        let _cleanup = workflow
            .begin_fault("recorder failed after readiness".into(), true)
            .expect("fault should claim cleanup while Starting");
        let stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop_polled);

        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            async move {
                stop_flag.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            },
            |directory| start_is_current(&workflow, directory),
            |_| true,
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(!stop_polled.load(Ordering::SeqCst));
        assert!(directory.exists());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        harness.shutdown();
    }

    #[tokio::test]
    async fn failed_start_cleanup_claim_blocks_late_fault_and_owns_one_stop() {
        let harness = Harness::new();
        let workflow = Rc::new(RefCell::new(harness.workflow()));
        let request = workflow
            .borrow_mut()
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        fs::write(&request.events_path, b"occupied")
            .expect("events path should force controller creation failure");
        let stop_calls = Rc::new(Cell::new(0));
        let stop_call_counter = Rc::clone(&stop_calls);
        let (stop_started_tx, stop_started_rx) = oneshot::channel();
        let (release_stop_tx, release_stop_rx) = oneshot::channel();
        let current_workflow = Rc::clone(&workflow);
        let cleanup_workflow = Rc::clone(&workflow);

        let cleanup = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            async move {
                stop_call_counter.set(stop_call_counter.get() + 1);
                stop_started_tx
                    .send(())
                    .expect("test should observe cleanup starting");
                release_stop_rx.await.expect("test should release cleanup");
                Ok(Vec::new())
            },
            move |directory| start_is_current(&current_workflow.borrow(), directory),
            move |directory| {
                cleanup_workflow
                    .borrow_mut()
                    .claim_failed_start_cleanup(directory)
            },
        );
        tokio::pin!(cleanup);
        tokio::select! {
            _ = &mut cleanup => panic!("cleanup completed before release"),
            started = stop_started_rx => started.expect("cleanup should start"),
        }

        let duplicate = handle_recorder_event(
            &mut workflow.borrow_mut(),
            RecorderEvent::Faulted {
                camera_id: Some(2),
                message: "late fatal recorder event".into(),
            },
        );

        assert!(duplicate.is_none(), "failed Start cleanup must own Stop");
        release_stop_tx
            .send(())
            .expect("cleanup future should still be waiting");
        let outcome = cleanup.await;
        apply_start_outcome(&mut workflow.borrow_mut(), outcome);

        assert_eq!(stop_calls.get(), 1);
        assert!(!directory.exists());
        assert!(!directory.join("recording-complete").exists());
        let state = workflow.borrow();
        assert!(matches!(state.session, SessionRunState::Idle));
        assert!(
            state
                .message
                .as_deref()
                .is_some_and(|message| message.contains("metadata start failed"))
        );
        drop(state);
        harness.shutdown();
    }

    #[tokio::test]
    async fn fault_winning_after_current_check_supersedes_failed_start_cleanup() {
        let harness = Harness::new();
        let workflow = Rc::new(RefCell::new(harness.workflow()));
        let request = workflow
            .borrow_mut()
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        fs::write(&request.events_path, b"occupied")
            .expect("events path should force controller creation failure");
        let stop_calls = Rc::new(Cell::new(0));
        let stop_call_counter = Rc::clone(&stop_calls);
        let current_workflow = Rc::clone(&workflow);
        let cleanup_workflow = Rc::clone(&workflow);

        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            async move {
                stop_call_counter.set(stop_call_counter.get() + 1);
                Ok(Vec::new())
            },
            move |directory| {
                assert!(start_is_current(&current_workflow.borrow(), directory));
                current_workflow
                    .borrow_mut()
                    .begin_fault("fault won cleanup ownership".into(), true)
                    .expect("fault should claim Starting");
                true
            },
            move |directory| {
                cleanup_workflow
                    .borrow_mut()
                    .claim_failed_start_cleanup(directory)
            },
        )
        .await;
        apply_start_outcome(&mut workflow.borrow_mut(), outcome);

        assert_eq!(stop_calls.get(), 0);
        assert!(directory.exists());
        assert!(!directory.join("recording-complete").exists());
        assert!(matches!(
            workflow.borrow().session,
            SessionRunState::Faulted { .. }
        ));
        harness.shutdown();
    }

    #[tokio::test]
    async fn controller_creation_failure_cleans_before_removing_staging() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        fs::write(&request.events_path, b"occupied")
            .expect("events path should force create_new failure");
        let cleanup_directory = directory.clone();
        let stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop_polled);

        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            async move {
                assert!(
                    cleanup_directory.exists(),
                    "staging must remain until recorder cleanup succeeds"
                );
                stop_flag.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            },
            |_| true,
            |directory| workflow.claim_failed_start_cleanup(directory),
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(stop_polled.load(Ordering::SeqCst));
        assert!(!directory.exists());
        assert!(matches!(workflow.session, SessionRunState::Idle));

        harness.shutdown();
    }

    #[tokio::test]
    async fn failed_start_cleanup_failure_preserves_staging_and_faults() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS)
            .expect("session should begin starting");
        let directory = request.directory.clone();
        fs::write(&request.events_path, b"occupied")
            .expect("events path should force create_new failure");

        let outcome = run_start_session_with(
            request,
            std::future::ready(Ok(())),
            std::future::ready(Err(RecorderError::FfmpegQuit)),
            |_| true,
            |directory| workflow.claim_failed_start_cleanup(directory),
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);

        assert!(directory.exists());
        assert!(matches!(
            &workflow.session,
            SessionRunState::Faulted { directory: actual, .. } if actual == &directory
        ));
        assert!(
            workflow
                .message
                .as_deref()
                .is_some_and(|message| message.contains("recorder cleanup failed"))
        );
        assert!(
            workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == backend::recording::RecorderStatus::Stopped)
        );
        assert!(!directory.join("recording-complete").exists());

        harness.shutdown();
    }

    #[tokio::test]
    async fn end_session_failure_still_polls_stop_and_never_marks_complete() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let directory = make_active(&mut workflow).await;
        let SessionRunState::Active { controller, .. } = &mut workflow.session else {
            panic!("session should be active");
        };
        controller
            .apply(OperatorAction::EndSession)
            .expect("test should pre-end the controller");
        let request = workflow
            .begin_stop()
            .expect("ended controller should still move to Stopping");
        let stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop_polled);

        let outcome = run_stop_session_with(request, async move {
            stop_flag.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .await;
        apply_stop_outcome(&mut workflow, outcome);

        assert!(stop_polled.load(Ordering::SeqCst));
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert!(!directory.join("recording-complete").exists());

        harness.shutdown();
    }

    #[tokio::test]
    async fn stop_failure_after_end_preserves_directory_without_marker() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let directory = make_active(&mut workflow).await;
        let events_path = directory.join("events.jsonl");
        let request = workflow
            .begin_stop()
            .expect("active session should begin stopping");

        let outcome =
            run_stop_session_with(request, std::future::ready(Err(RecorderError::FfmpegQuit)))
                .await;
        apply_stop_outcome(&mut workflow, outcome);

        Session::load(&events_path).expect("EndSession should remain durable");
        assert!(directory.exists());
        assert!(!directory.join("recording-complete").exists());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert!(
            workflow
                .message
                .as_deref()
                .is_some_and(|message| message.contains("Recorder Stop failed"))
        );
        assert!(
            workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == backend::recording::RecorderStatus::Stopped)
        );

        harness.shutdown();
    }

    #[tokio::test]
    async fn completion_marker_failure_keeps_workflow_faulted() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let directory = make_active(&mut workflow).await;
        fs::write(directory.join("recording-complete"), b"")
            .expect("existing marker should force create_new failure");
        let request = workflow
            .begin_stop()
            .expect("active session should begin stopping");

        let outcome = run_stop_session_with(request, std::future::ready(Ok(Vec::new()))).await;
        apply_stop_outcome(&mut workflow, outcome);

        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert!(workflow.sessions.is_empty());
        harness.shutdown();
    }

    #[tokio::test]
    async fn uncertain_append_fault_skips_second_end_but_still_polls_cleanup() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let directory = make_active(&mut workflow).await;
        let events_path = directory.join("events.jsonl");
        let error = workflow
            .set_sampling_interval(2, Duration::ZERO)
            .expect_err("invalid append should fail");
        let request = workflow
            .begin_fault(error.to_string(), false)
            .expect("metadata failure should claim cleanup");
        assert!(request.controller.is_none());
        let stop_polled = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop_polled);

        let outcome = run_fault_session_with(request, async move {
            stop_flag.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .await;
        apply_fault_outcome(&mut workflow, outcome);

        assert!(stop_polled.load(Ordering::SeqCst));
        assert!(Session::load(&events_path).is_err());
        assert!(!directory.join("recording-complete").exists());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));

        harness.shutdown();
    }

    #[tokio::test]
    async fn fatal_cleanup_appends_end_before_stop_without_marking_complete() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let directory = make_active(&mut workflow).await;
        let events_path = directory.join("events.jsonl");
        let request = workflow
            .begin_fault("fatal recorder error".into(), true)
            .expect("active fatal event should claim cleanup");
        assert!(request.controller.is_some());
        let stop_events = events_path.clone();

        let outcome = run_fault_session_with(request, async move {
            Session::load(&stop_events)
                .expect("fatal cleanup should append EndSession before Stop");
            Ok(Vec::new())
        })
        .await;
        apply_fault_outcome(&mut workflow, outcome);

        assert!(Session::load(&events_path).is_ok());
        assert!(!directory.join("recording-complete").exists());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert_eq!(workflow.message.as_deref(), Some("fatal recorder error"));
        assert!(
            workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status == backend::recording::RecorderStatus::Stopped)
        );

        harness.shutdown();
    }

    #[tokio::test]
    async fn fatal_recorder_event_sets_shared_message_and_named_camera_health() {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        make_active(&mut workflow).await;

        let cleanup = handle_recorder_event(
            &mut workflow,
            RecorderEvent::Faulted {
                camera_id: Some(2),
                message: "camera recorder failed".into(),
            },
        )
        .expect("fatal event should claim cleanup");

        assert_eq!(workflow.message.as_deref(), Some("camera recorder failed"));
        assert_eq!(
            workflow.cameras[0].recorder_status,
            backend::recording::RecorderStatus::Recording
        );
        assert_eq!(
            workflow.cameras[1].recorder_status,
            backend::recording::RecorderStatus::Stopped
        );
        drop(cleanup);
        harness.shutdown();
    }
}
