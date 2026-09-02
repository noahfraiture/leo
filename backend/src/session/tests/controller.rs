use std::{fs, time::Duration};

use serde_json::{Value, json};
use uuid::Uuid;

use super::{OperatorAction, SessionController};
use crate::session::SessionCamera;

fn camera(camera_id: u32, sample_every: Duration) -> SessionCamera {
    SessionCamera {
        id: camera_id,
        name: format!("Camera {camera_id}"),
        enabled: true,
        sample_every,
    }
}

fn read_events(path: &std::path::Path) -> Vec<Value> {
    let contents = fs::read_to_string(path).expect("events should be readable");
    assert!(contents.ends_with('\n'));
    contents
        .lines()
        .map(|line| serde_json::from_str(line).expect("event should be JSON"))
        .collect()
}

#[test]
fn create_writes_the_session_start_schema_immediately() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");

    let _controller = SessionController::create(
        path.clone(),
        vec![SessionCamera {
            id: 7,
            name: "Front".into(),
            enabled: false,
            sample_every: Duration::from_millis(2_500),
        }],
    )
    .expect("session should be created");

    let events = read_events(&path);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["schema_version"], 1);
    assert_eq!(events[0]["sequence"], 0);
    assert_eq!(events[0]["session_offset_ms"], 0);
    assert!(events[0]["utc_ms"].as_i64().is_some_and(|utc| utc > 0));
    Uuid::parse_str(
        events[0]["session_id"]
            .as_str()
            .expect("session ID should be a string"),
    )
    .expect("session ID should be a UUID");
    assert_eq!(
        events[0]["action"],
        json!({
            "type": "session_started",
            "cameras": [{
                "camera_id": 7,
                "name": "Front",
                "enabled": false,
                "sample_every_ms": 2_500
            }]
        })
    );
}

#[test]
fn apply_routes_actions_and_assigns_contiguous_sequences() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller =
        SessionController::create(path.clone(), vec![camera(1, Duration::from_secs(5))])
            .expect("session should be created");

    controller
        .apply(OperatorAction::SetCameraParticipation {
            camera_id: 1,
            enabled: false,
        })
        .expect("participation should be recorded");
    controller
        .apply(OperatorAction::SetSamplingInterval {
            camera_id: 1,
            sample_every: Duration::from_millis(750),
        })
        .expect("sampling interval should be recorded");
    controller
        .apply(OperatorAction::EndSession)
        .expect("session end should be recorded");

    let events = read_events(&path);
    assert_eq!(events.len(), 4);
    assert_eq!(
        events
            .iter()
            .map(|event| event["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0]["session_offset_ms"].as_u64()
                <= pair[1]["session_offset_ms"].as_u64())
    );
    assert!(
        events
            .iter()
            .all(|event| event["session_id"] == events[0]["session_id"])
    );
    assert_eq!(
        events[1]["action"],
        json!({
            "type": "camera_participation_changed",
            "camera_id": 1,
            "enabled": false
        })
    );
    assert_eq!(
        events[2]["action"],
        json!({
            "type": "sampling_interval_changed",
            "camera_id": 1,
            "sample_every_ms": 750
        })
    );
    assert_eq!(events[3]["action"], json!({"type": "session_ended"}));
}

#[test]
fn create_rejects_invalid_camera_lists_before_writing() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");

    for (name, cameras, expected) in [
        ("empty", vec![], "at least one camera"),
        (
            "zero ID",
            vec![camera(0, Duration::from_secs(1))],
            "camera ID must be non-zero",
        ),
        (
            "duplicate ID",
            vec![
                camera(1, Duration::from_secs(1)),
                camera(1, Duration::from_secs(2)),
            ],
            "duplicate camera 1",
        ),
        (
            "zero interval",
            vec![camera(1, Duration::ZERO)],
            "sampling interval",
        ),
    ] {
        let path = directory.path().join(format!("{name}.jsonl"));

        let error = SessionController::create(path.clone(), cameras)
            .expect_err("invalid cameras should be rejected");

        assert!(error.to_string().contains(expected), "{name}");
        assert!(!path.exists(), "{name}");
    }
}

#[test]
fn apply_rejects_zero_intervals_without_appending() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller =
        SessionController::create(path.clone(), vec![camera(1, Duration::from_secs(1))])
            .expect("session should be created");

    let error = controller
        .apply(OperatorAction::SetSamplingInterval {
            camera_id: 1,
            sample_every: Duration::ZERO,
        })
        .expect_err("sampling intervals should be positive");

    assert!(error.to_string().contains("sampling interval"));
    assert_eq!(read_events(&path).len(), 1);
}

#[test]
fn apply_rejects_unknown_cameras_without_appending() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller =
        SessionController::create(path.clone(), vec![camera(1, Duration::from_secs(1))])
            .expect("session should be created");

    for action in [
        OperatorAction::SetCameraParticipation {
            camera_id: 9,
            enabled: false,
        },
        OperatorAction::SetSamplingInterval {
            camera_id: 9,
            sample_every: Duration::from_secs(1),
        },
    ] {
        let error = controller
            .apply(action)
            .expect_err("camera should belong to the session");
        assert!(error.to_string().contains("unknown camera 9"));
    }

    assert_eq!(read_events(&path).len(), 1);
}

#[test]
fn apply_rejects_actions_after_the_session_ends() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller =
        SessionController::create(path.clone(), vec![camera(1, Duration::from_secs(1))])
            .expect("session should be created");
    controller
        .apply(OperatorAction::EndSession)
        .expect("session should end");

    let error = controller
        .apply(OperatorAction::SetCameraParticipation {
            camera_id: 1,
            enabled: false,
        })
        .expect_err("ended sessions should reject actions");

    assert!(error.to_string().contains("session has ended"));
    assert_eq!(read_events(&path).len(), 2);
}

#[test]
fn elapsed_advances_with_the_session_clock() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let controller = SessionController::create(path, vec![camera(1, Duration::from_secs(1))])
        .expect("session should be created");
    let first = controller.elapsed();

    std::thread::sleep(Duration::from_millis(1));

    assert!(controller.elapsed() > first);
}
