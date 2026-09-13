use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use backend::{
    recording::{RecorderEvent, RecorderRuntime, RecorderSettings, RecorderStatus, test_support},
    session::{OperatorAction, SessionController, mark_recording_complete},
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use super::{Error, OperatorState, SessionRunState};
use crate::settings::CameraSettings;

const START_UTC_MS: i64 = 1_786_552_800_000;

struct Harness {
    _temporary: TempDir,
    runtime: Option<RecorderRuntime>,
    workflow: OperatorState,
}

impl Harness {
    fn new() -> Self {
        Self::with(camera_settings())
    }

    fn with(cameras: Vec<CameraSettings>) -> Self {
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
        let workflow = OperatorState::new(
            crate::test_settings(cameras, None, 5, 0),
            temporary.path().join("sessions"),
            recorder,
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

fn start_active(workflow: &mut OperatorState) -> PathBuf {
    let request = workflow
        .begin_start(START_UTC_MS)
        .expect("idle workflow should begin starting");
    let controller = SessionController::create(
        request.events_path.clone(),
        request.session_cameras.clone(),
        crate::test_monitoring_profiles(),
    )
    .expect("session controller should be created");
    let directory = request.directory.clone();
    workflow.finish_start(directory.clone(), Some(controller));
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
            "schema_version": 2,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started", "monitoring_profiles": crate::test_monitoring_profiles(),
                "cameras": [{
                    "camera_id": 1,
                    "name": "Salon 1",
                    "enabled": true,
                    "initial_monitoring_profile_id": 1
                }]
            }
        }),
        json!({
            "schema_version": 2,
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
    fs::write(directory.join("events.jsonl"), events).expect("session events should be written");
    if marked {
        mark_recording_complete(&directory).expect("session should be marked complete");
    }
    directory
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
            .map(|camera| (
                camera.id,
                camera.enabled,
                camera.initial_monitoring_profile_id
            ))
            .collect::<Vec<_>>(),
        [(1, true, 1), (2, false, 2),]
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
    let mut harness = Harness::with(Vec::new());
    let root = harness.workflow.session_root.clone();

    assert!(matches!(
        harness.workflow.begin_start(123),
        Err(Error::NoCamerasConfigured)
    ));
    assert!(!root.exists());
    harness.shutdown();
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
        .as_mut()
        .unwrap()
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

    let controller = SessionController::create(
        request.events_path,
        request.session_cameras,
        crate::test_monitoring_profiles(),
    )
    .expect("controller should be created");
    harness
        .workflow
        .finish_start(request.directory, Some(controller));
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
    let controller = SessionController::create(
        request.events_path,
        request.session_cameras,
        crate::test_monitoring_profiles(),
    )
    .expect("late session controller should be constructible");
    harness
        .workflow
        .finish_start(request.directory, Some(controller));

    assert!(matches!(
        harness.workflow.session,
        SessionRunState::Faulted { .. }
    ));

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
    let controller = SessionController::create(
        request.events_path,
        request.session_cameras,
        crate::test_monitoring_profiles(),
    )
    .expect("session controller should start");

    harness
        .workflow
        .finish_start(request.directory, Some(controller));

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
        crate::test_monitoring_profiles(),
    )
    .expect("mismatched controller should be created for the failure test");
    harness
        .workflow
        .finish_start(request.directory, Some(controller));

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
        .set_monitoring_profile(vec![2], 0)
        .expect_err("controller must reject zero cadence");

    assert_eq!(harness.workflow.cameras[1].active_monitoring_profile_id, 2);
    let cleanup = harness
        .workflow
        .begin_fault(error.to_string(), false)
        .expect("write failure should produce cleanup");
    assert!(cleanup.controller.is_none());

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
