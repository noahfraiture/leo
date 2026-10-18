use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use backend::{
    recording::{
        Error as RecorderError, RecorderEvent, RecorderHandle, RecorderRuntime, RecorderSettings,
        test_support,
    },
    session::{OperatorAction, Session},
};

use super::{
    apply_fault_outcome, apply_start_outcome, apply_stop_outcome, handle_recorder_event,
    handle_recorder_event_channel_closed, run_fault_session_with, run_start_session_with,
    run_stop_session_with, run_storage, start_is_current,
};
use crate::{
    operator::{OperatorState, SessionRunState},
    settings::CameraSettings,
};

const START_UTC_MS: i64 = 1_786_552_800_000;

#[tokio::test(flavor = "current_thread")]
async fn session_storage_waits_leave_the_ui_timer_responsive() {
    let writing = run_storage(|| {
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    });
    tokio::pin!(writing);

    tokio::select! {
        biased;
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        _ = &mut writing => panic!("Session storage blocked the UI timer"),
    }
    writing.await.unwrap();
}

#[tokio::test]
async fn a_session_storage_worker_panic_returns_a_typed_failure() {
    let error = run_storage::<()>(|| panic!("controlled storage worker failure"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        super::super::error::SessionStorageError::Worker(_)
    ));
}

#[tokio::test]
async fn stop_retains_both_completion_and_discovery_failures() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let directory = make_active(&mut workflow).await;
    let session_root = workflow.session_root.clone();
    let retained = session_root.with_extension("offline");
    let request = workflow.begin_stop().unwrap();

    let outcome = run_stop_session_with(request, async {
        // Simulate a storage mount changing after the end event without deleting captured files.
        fs::rename(&session_root, &retained).unwrap();
        fs::write(&session_root, b"storage unavailable").unwrap();
        Ok(Vec::new())
    })
    .await;
    apply_stop_outcome(&mut workflow, outcome);

    assert!(matches!(workflow.session, SessionRunState::Idle));
    assert!(
        retained
            .join(directory.file_name().unwrap())
            .join("events.jsonl")
            .is_file()
    );
    let message = workflow.message.as_deref().unwrap();
    assert!(message.contains("Completion marker failed"), "{message}");
    assert!(
        message.contains("Session list could not be refreshed"),
        "{message}"
    );
    harness.shutdown();
}

struct Harness {
    temporary: tempfile::TempDir,
    runtime: Option<RecorderRuntime>,
    recorder: RecorderHandle,
}

impl Harness {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let (runtime, recorder, _events) = test_support::spawn(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            PathBuf::from("unused-test-ffmpeg"),
            PathBuf::from("unused-test-ffprobe"),
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

    fn workflow(&self) -> OperatorState {
        OperatorState::new(
            crate::test_settings(camera_settings(), Some(crate::test_openai_config()), 5, 0),
            self.session_root(),
            self.recorder.clone(),
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

fn camera_settings() -> Vec<CameraSettings> {
    vec![
        CameraSettings {
            id: 1,
            name: "Salon 1".into(),
            rtsp_url: "rtsp://camera-one.example/live".into(),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: 1,
        },
        CameraSettings {
            id: 2,
            name: "Salon 2".into(),
            rtsp_url: "rtsp://camera-two.example/live".into(),
            initially_included_in_analysis: false,
            initial_monitoring_profile_id: 2,
        },
    ]
}

async fn make_active(operator: &mut OperatorState) -> PathBuf {
    let request = operator
        .begin_start(START_UTC_MS)
        .expect("session should begin starting");
    let directory = request.directory.clone();
    let outcome = run_start_session_with(request, std::future::ready(Ok(())), |_| true).await;
    apply_start_outcome(operator, outcome);
    assert!(matches!(operator.session, SessionRunState::Active { .. }));
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
        let recordings = start_events_path.parent().unwrap().join("recordings");
        for camera in [1, 2] {
            assert!(recordings.join(format!("camera-{camera}")).is_dir());
        }
        Ok(())
    };
    let outcome = run_start_session_with(request, start, |_| true).await;
    apply_start_outcome(&mut workflow, outcome);

    assert!(matches!(workflow.session, SessionRunState::Active { .. }));
    workflow
        .set_monitoring_profile(vec![2], 3)
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
        Session::load(&stop_events_path).expect("end event must precede recorder Stop");
        assert!(
            !stop_marker.exists(),
            "completion follows media finalization"
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
                OperatorAction::SetMonitoringProfile {
                    camera_ids: vec![2],
                    monitoring_profile_id: 3
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

    let outcome = run_start_session_with(
        request,
        std::future::ready(Err(RecorderError::RecorderStartupFailed)),
        |_| true,
    )
    .await;
    apply_start_outcome(&mut workflow, outcome);

    assert!(matches!(workflow.session, SessionRunState::Idle));
    assert!(!directory.exists());
    assert!(!events_path.exists());
    assert!(workflow.message.is_some());

    harness.shutdown();
}

#[tokio::test]
async fn staging_collision_preserves_existing_files_and_never_starts_the_recorder() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let directory = workflow.session_root.join(START_UTC_MS.to_string());
    fs::create_dir_all(&directory).unwrap();
    let retained = directory.join("existing-recording.mkv");
    fs::write(&retained, b"existing bytes").unwrap();
    let request = workflow.begin_start(START_UTC_MS).unwrap();
    let started = AtomicBool::new(false);

    let outcome = run_start_session_with(
        request,
        async {
            started.store(true, Ordering::SeqCst);
            Ok(())
        },
        |_| true,
    )
    .await;
    apply_start_outcome(&mut workflow, outcome);

    assert!(!started.load(Ordering::SeqCst));
    assert!(matches!(workflow.session, SessionRunState::Idle));
    assert_eq!(fs::read(retained).unwrap(), b"existing bytes");
    assert!(
        workflow
            .message
            .as_deref()
            .unwrap()
            .contains("Session start storage failed")
    );
    harness.shutdown();
}

#[tokio::test]
async fn superseded_recorder_failure_preserves_the_fault_owned_staging_folder() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let request = workflow.begin_start(START_UTC_MS).unwrap();
    let retained = request.directory.join("fault-diagnostic.mkv");
    let current = Cell::new(true);

    let outcome = run_start_session_with(
        request,
        async {
            fs::write(&retained, b"retained fault evidence").unwrap();
            current.set(false);
            Err(RecorderError::RecorderStartupFailed)
        },
        |_| current.get(),
    )
    .await;

    assert_eq!(fs::read(retained).unwrap(), b"retained fault evidence");
    assert!(matches!(outcome, super::StartOutcome::Superseded));
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

    let started = AtomicBool::new(false);
    let outcome = run_start_session_with(
        request,
        async {
            started.store(true, Ordering::SeqCst);
            Ok(())
        },
        |directory| start_is_current(&workflow, directory),
    )
    .await;
    apply_start_outcome(&mut workflow, outcome);

    assert!(!events_path.exists());
    assert!(!started.load(Ordering::SeqCst));
    assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
    harness.shutdown();
}

#[tokio::test]
async fn superseded_start_outcomes_cannot_replace_a_newer_fault_warning() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let request = workflow.begin_start(START_UTC_MS).unwrap();
    workflow.begin_fault("recorder fault".into(), true).unwrap();
    workflow.metadata_error = Some("retained metadata warning".into());

    for outcome in [
        super::StartOutcome::Active {
            directory: request.directory.clone(),
            controller: None,
            metadata_error: None,
        },
        super::StartOutcome::Idle {
            directory: request.directory.clone(),
            message: "late startup failure".into(),
        },
        super::StartOutcome::Faulted {
            directory: request.directory,
            message: "late startup cleanup failure".into(),
        },
    ] {
        apply_start_outcome(&mut workflow, outcome);
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert_eq!(workflow.message.as_deref(), Some("recorder fault"));
        assert_eq!(
            workflow.metadata_error.as_deref(),
            Some("retained metadata warning")
        );
    }
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
async fn recorder_channel_closure_while_idle_blocks_start_after_other_actions() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    assert!(handle_recorder_event_channel_closed(&mut workflow).is_none());

    workflow.select_camera(2).unwrap();

    assert!(workflow.begin_start(START_UTC_MS).is_err());
    assert!(!workflow.session_root.exists());
    harness.shutdown();
}

#[tokio::test]
async fn recorder_channel_closure_during_finalization_keeps_future_starts_unavailable() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let directory = make_active(&mut workflow).await;
    let request = workflow.begin_stop().unwrap();
    let outcome = run_stop_session_with(request, std::future::ready(Ok(Vec::new()))).await;

    // Recorder Stop succeeded, but its runtime disappears before the UI accepts finalization.
    assert!(handle_recorder_event_channel_closed(&mut workflow).is_none());
    apply_stop_outcome(&mut workflow, outcome);
    workflow.select_camera(2).unwrap();

    assert!(matches!(workflow.session, SessionRunState::Idle));
    assert!(directory.join("recording-complete").is_file());
    assert_eq!(workflow.sessions.len(), 1);
    assert!(workflow.begin_start(START_UTC_MS + 1).is_err());
    harness.shutdown();
}

#[tokio::test]
async fn unavailable_start_metadata_preserves_capture_and_allows_the_next_session() {
    for invalid_profiles in [false, true] {
        let harness = Harness::new();
        let mut workflow = harness.workflow();
        let mut request = workflow.begin_start(START_UTC_MS).unwrap();
        let directory = request.directory.clone();
        let media = directory.join("recordings/camera-1/retained.mkv");
        if invalid_profiles {
            request.metadata_error = Some("invalid monitoring configuration".into());
        }
        let captured = media.clone();
        let events_path = request.events_path.clone();
        let outcome = run_start_session_with(
            request,
            async move {
                fs::write(&captured, b"captured bytes").unwrap();
                if !invalid_profiles {
                    fs::write(events_path, b"occupied").unwrap();
                }
                Ok(())
            },
            |_| true,
        )
        .await;
        apply_start_outcome(&mut workflow, outcome);
        assert!(matches!(
            workflow.session,
            SessionRunState::Active {
                controller: None,
                ..
            }
        ));
        assert!(
            workflow
                .metadata_error
                .as_ref()
                .unwrap()
                .contains("Recording continues")
        );
        assert_eq!(fs::read(&media).unwrap(), b"captured bytes");
        let request = workflow.begin_stop().unwrap();
        let outcome = run_stop_session_with(request, std::future::ready(Ok(Vec::new()))).await;
        apply_stop_outcome(&mut workflow, outcome);
        assert!(matches!(workflow.session, SessionRunState::Idle));
        assert!(!directory.join("recording-complete").exists());
        assert!(workflow.incomplete_sessions.contains(&directory));
        workflow
            .begin_start(START_UTC_MS + 1)
            .expect("metadata repair must not block the next customer");
        harness.shutdown();
    }
}

#[tokio::test]
async fn capture_fault_after_metadata_failure_still_claims_cleanup_once() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let mut request = workflow.begin_start(START_UTC_MS).unwrap();
    request.metadata_error = Some("monitoring unavailable".into());
    let outcome = run_start_session_with(request, std::future::ready(Ok(())), |_| true).await;
    apply_start_outcome(&mut workflow, outcome);
    let fault = RecorderEvent::Faulted {
        camera_id: Some(1),
        message: "capture failed".into(),
    };
    let cleanup = handle_recorder_event(&mut workflow, fault)
        .expect("real capture failure still requests cleanup");
    assert!(cleanup.controller.is_none());
    assert!(handle_recorder_event_channel_closed(&mut workflow).is_none());
    let outcome = run_fault_session_with(cleanup, std::future::ready(Ok(Vec::new()))).await;
    apply_fault_outcome(&mut workflow, outcome);
    assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
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
        .as_mut()
        .unwrap()
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
    assert!(matches!(workflow.session, SessionRunState::Idle));
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
        run_stop_session_with(request, std::future::ready(Err(RecorderError::FfmpegQuit))).await;
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
async fn completion_marker_failure_retains_recording_and_allows_a_new_session() {
    let harness = Harness::new();
    let mut workflow = harness.workflow();
    let directory = make_active(&mut workflow).await;
    fs::write(directory.join("recording-complete"), b"incomplete")
        .expect("existing marker should force create_new failure");
    let request = workflow
        .begin_stop()
        .expect("active session should begin stopping");

    let outcome = run_stop_session_with(request, std::future::ready(Ok(Vec::new()))).await;
    apply_stop_outcome(&mut workflow, outcome);

    assert!(matches!(workflow.session, SessionRunState::Idle));
    assert!(workflow.sessions.is_empty());
    assert!(workflow.incomplete_sessions.contains(&directory));
    workflow.begin_start(START_UTC_MS + 1).unwrap();
    harness.shutdown();
}

#[tokio::test]
async fn failed_metadata_append_preserves_last_saved_selections_and_recording() {
    let harness = Harness::new();
    for participation in [false, true] {
        let mut workflow = harness.workflow();
        let request = workflow
            .begin_start(START_UTC_MS + i64::from(participation))
            .unwrap();
        let directory = request.directory.clone();
        let outcome = run_start_session_with(request, std::future::ready(Ok(())), |_| true).await;
        apply_start_outcome(&mut workflow, outcome);
        let SessionRunState::Active {
            controller: Some(controller),
            ..
        } = &mut workflow.session
        else {
            panic!("active writer required")
        };
        controller.fail_writes_for_test().unwrap();
        let result = if participation {
            workflow.set_participation(2, true)
        } else {
            workflow.set_monitoring_profile(vec![1, 2], 3)
        };
        assert!(result.is_err());
        assert!(matches!(
            workflow.session,
            SessionRunState::Active {
                controller: None,
                ..
            }
        ));
        assert_eq!(workflow.cameras[0].active_monitoring_profile_id, 1);
        assert_eq!(workflow.cameras[1].active_monitoring_profile_id, 2);
        assert!(!workflow.cameras[1].participating);
        assert!(
            workflow
                .cameras
                .iter()
                .all(|camera| camera.recorder_status
                    == backend::recording::RecorderStatus::Recording)
        );
        assert!(workflow.set_monitoring_profile(vec![1], 2).is_err());
        let request = workflow.begin_stop().unwrap();
        let outcome = run_stop_session_with(request, std::future::ready(Ok(Vec::new()))).await;
        apply_stop_outcome(&mut workflow, outcome);
        assert!(matches!(workflow.session, SessionRunState::Idle));
        assert!(!directory.join("recording-complete").exists());
    }
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
