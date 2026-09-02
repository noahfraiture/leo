use std::{
    cell::{Cell, RefCell},
    fs,
    num::NonZeroUsize,
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
        Error as RecorderError, RecorderEvent, RecorderHandle, RecorderRuntime, RecorderSettings,
        test_support,
    },
    session::{OperatorAction, Session},
};
use tokio::sync::oneshot;

use super::{
    apply_fault_outcome, apply_start_outcome, apply_stop_outcome, handle_recorder_event,
    handle_recorder_event_channel_closed, run_fault_session_with, run_start_session_with,
    run_stop_session_with, start_is_current,
};
use crate::{
    operator::{OperatorState, SessionRunState},
    settings::CameraSettings,
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
            camera_settings(),
            self.session_root(),
            self.recorder.clone(),
            Some(crate::test_openai_config()),
            NonZeroUsize::new(5).unwrap(),
            0,
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

async fn make_active(operator: &mut OperatorState) -> PathBuf {
    let request = operator
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
        Ok(())
    };
    let failed_start_stop_polled = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&failed_start_stop_polled);
    let unused_cleanup = async move {
        stop_flag.store(true, Ordering::SeqCst);
        Ok(Vec::new())
    };

    let outcome = run_start_session_with(request, start, unused_cleanup, |_| true, |_| true).await;
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
