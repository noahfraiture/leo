use std::{
    fs,
    num::NonZeroUsize,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
    time::Duration,
};

use backend::{
    recording::{RecorderEvent, RecorderSettings, RecorderStatus, spawn_for_test},
    session::SessionController,
};

use super::bootstrap::{InitialWorkflow, RecorderBootstrap, initialize_workflow_for_test};
use crate::{
    session_task::handle_recorder_event,
    settings::{CameraSettings, Settings, SettingsStore},
    workflow::{SessionRunState, Workflow},
};

fn camera_settings() -> Vec<CameraSettings> {
    [1_u32, 2]
        .into_iter()
        .map(|id| CameraSettings {
            id,
            name: format!("Salon {id}"),
            rtsp_url: format!("rtsp://camera-{id}.example/live"),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        })
        .collect()
}

fn test_recorder() -> (
    tempfile::TempDir,
    backend::recording::RecorderRuntime,
    RecorderBootstrap,
) {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let executable = temporary.path().join("successful-preflight");
    fs::write(&executable, "#!/bin/sh\nexit 0\n")
        .expect("fake preflight executable should be written");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("fake preflight executable should be executable");
    let (runtime, handle, events) = spawn_for_test(
        RecorderSettings {
            io_timeout: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(1),
        },
        executable.clone(),
        executable,
    )
    .expect("test recorder runtime should start");
    (
        temporary,
        runtime,
        RecorderBootstrap {
            handle,
            events: Arc::new(Mutex::new(Some(events))),
        },
    )
}

#[test]
fn recorder_event_receiver_is_taken_exactly_once() {
    let (_temporary, runtime, recorder) = test_recorder();
    let mut events = recorder
        .events
        .lock()
        .expect("recorder event receiver mutex should not be poisoned");

    assert!(events.take().is_some());
    assert!(events.take().is_none());
    drop(events);

    runtime.shutdown().expect("runtime should shut down");
}

#[test]
fn workflow_bootstrap_reports_catalogue_io_without_taking_receiver() {
    let (temporary, runtime, recorder) = test_recorder();
    let data_root = temporary.path().join("data");
    fs::create_dir(&data_root).expect("data root should be created");
    let settings = Settings {
        next_camera_id: 3,
        cameras: camera_settings(),
        ..Settings::default()
    };
    let store = SettingsStore::new(temporary.path().join("config/settings.json"), data_root);
    let config = store
        .resolve(settings)
        .expect("test settings should resolve");
    fs::write(&config.sessions_root, b"not a directory")
        .expect("catalogue root should become invalid after startup validation");

    let Err(error) = initialize_workflow_for_test(&config, &recorder) else {
        panic!("invalid catalogue should make Workflow unavailable");
    };

    assert!(error.contains("Session workflow is unavailable"));
    assert!(
        recorder
            .events
            .lock()
            .expect("recorder event receiver mutex should not be poisoned")
            .take()
            .is_some()
    );
    runtime.shutdown().expect("runtime should shut down");
}

#[test]
fn workflow_bootstrap_copies_active_analysis_batching_settings() {
    let (temporary, runtime, recorder) = test_recorder();
    let store = SettingsStore::new(
        temporary.path().join("config/settings.json"),
        temporary.path().join("data"),
    );
    let config = store
        .resolve(Settings {
            analysis_frame_sets_per_prompt: 7,
            analysis_overlap_frame_sets: 2,
            ..Settings::default()
        })
        .expect("test settings should resolve");

    let InitialWorkflow(initial) =
        initialize_workflow_for_test(&config, &recorder).expect("workflow should initialize");
    let workflow = initial
        .lock()
        .expect("initial workflow mutex should not be poisoned")
        .take()
        .expect("workflow should be retained");

    assert_eq!(workflow.analysis_frame_sets_per_prompt.get(), 7);
    assert_eq!(workflow.analysis_overlap_frame_sets, 2);
    runtime.shutdown().expect("runtime should shut down");
}

#[test]
fn root_event_dispatch_updates_reconnecting_and_claims_one_fatal_cleanup() {
    let (temporary, runtime, recorder) = test_recorder();
    let mut workflow = Workflow::new(
        camera_settings(),
        temporary.path().join("sessions"),
        recorder.handle.clone(),
        Some(crate::test_openai_config()),
        NonZeroUsize::new(5).unwrap(),
        0,
    )
    .expect("workflow should initialize");
    let request = workflow
        .begin_start(1_786_552_800_000)
        .expect("session should begin starting");
    let controller = SessionController::create(request.events_path, request.session_cameras)
        .expect("session controller should start");
    workflow.finish_start(request.directory, controller);

    assert!(
        handle_recorder_event(
            &mut workflow,
            RecorderEvent::Status {
                camera_id: 2,
                status: RecorderStatus::Reconnecting,
                message: Some("camera stream interrupted".into()),
            },
        )
        .is_none()
    );
    assert_eq!(
        workflow.cameras[1].recorder_status,
        RecorderStatus::Reconnecting
    );
    assert!(matches!(workflow.session, SessionRunState::Active { .. }));

    let cleanup = handle_recorder_event(
        &mut workflow,
        RecorderEvent::Faulted {
            camera_id: Some(2),
            message: "recorder storage failed".into(),
        },
    )
    .expect("first fatal event should claim cleanup");
    assert!(cleanup.controller.is_some());
    assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
    assert!(
        handle_recorder_event(
            &mut workflow,
            RecorderEvent::Faulted {
                camera_id: Some(1),
                message: "duplicate fatal event".into(),
            },
        )
        .is_none()
    );

    runtime.shutdown().expect("runtime should shut down");
}
