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
        initial_monitoring_profile_id: sample_every.as_millis() as u32,
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
            initial_monitoring_profile_id: 2_500,
        }],
        crate::tests::monitoring_profiles(),
    )
    .expect("session should be created");

    let events = read_events(&path);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["schema_version"], 2);
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
            "type": "session_started", "monitoring_profiles": crate::tests::monitoring_profiles(),
            "cameras": [{
                "camera_id": 7,
                "name": "Front",
                "enabled": false,
                "initial_monitoring_profile_id": 2_500
            }]
        })
    );
}

#[test]
fn apply_routes_actions_and_assigns_contiguous_sequences() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller = SessionController::create(
        path.clone(),
        vec![camera(1, Duration::from_secs(5))],
        crate::tests::monitoring_profiles(),
    )
    .expect("session should be created");

    controller
        .apply(OperatorAction::SetCameraParticipation {
            camera_id: 1,
            enabled: false,
        })
        .expect("participation should be recorded");
    controller
        .apply(OperatorAction::SetMonitoringProfile {
            camera_ids: vec![1],
            monitoring_profile_id: 750,
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
            "type": "monitoring_profile_changed", "camera_ids": [1], "monitoring_profile_id": 750
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
            "monitoring profile",
        ),
    ] {
        let path = directory.path().join(format!("{name}.jsonl"));

        let error =
            SessionController::create(path.clone(), cameras, crate::tests::monitoring_profiles())
                .expect_err("invalid cameras should be rejected");

        assert!(error.to_string().contains(expected), "{name}");
        assert!(!path.exists(), "{name}");
    }
}

#[test]
fn apply_rejects_unknown_profiles_without_appending() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller = SessionController::create(
        path.clone(),
        vec![camera(1, Duration::from_secs(1))],
        crate::tests::monitoring_profiles(),
    )
    .expect("session should be created");

    let error = controller
        .apply(OperatorAction::SetMonitoringProfile {
            camera_ids: vec![1],
            monitoring_profile_id: 0,
        })
        .expect_err("profile must belong to the snapshot");

    assert!(error.to_string().contains("monitoring profile"));
    assert_eq!(read_events(&path).len(), 1);
}

#[test]
fn apply_rejects_unknown_cameras_without_appending() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let mut controller = SessionController::create(
        path.clone(),
        vec![camera(1, Duration::from_secs(1))],
        crate::tests::monitoring_profiles(),
    )
    .expect("session should be created");

    for action in [
        OperatorAction::SetCameraParticipation {
            camera_id: 9,
            enabled: false,
        },
        OperatorAction::SetMonitoringProfile {
            camera_ids: vec![9],
            monitoring_profile_id: 1000,
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
    let mut controller = SessionController::create(
        path.clone(),
        vec![camera(1, Duration::from_secs(1))],
        crate::tests::monitoring_profiles(),
    )
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
    let controller = SessionController::create(
        path,
        vec![camera(1, Duration::from_secs(1))],
        crate::tests::monitoring_profiles(),
    )
    .expect("session should be created");
    let first = controller.elapsed();

    std::thread::sleep(Duration::from_millis(1));

    assert!(controller.elapsed() > first);
}

#[test]
fn bulk_profile_assignment_validates_every_target_before_one_durable_event() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("events.jsonl");
    let profiles = crate::tests::monitoring_profiles();
    let mut controller = SessionController::create(
        path.clone(),
        vec![
            camera(1, Duration::from_secs(1)),
            camera(2, Duration::from_secs(2)),
        ],
        profiles.clone(),
    )
    .unwrap();
    let initial = std::fs::read(&path).unwrap();
    for camera_ids in [vec![], vec![1, 1], vec![1, 9]] {
        assert!(
            controller
                .apply(OperatorAction::SetMonitoringProfile {
                    camera_ids,
                    monitoring_profile_id: 500
                })
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), initial);
    }
    controller
        .apply(OperatorAction::SetMonitoringProfile {
            camera_ids: vec![1, 2],
            monitoring_profile_id: 500,
        })
        .unwrap();
    controller.apply(OperatorAction::EndSession).unwrap();
    let session = crate::session::Session::load(&path).unwrap();
    assert_eq!(session.monitoring_profiles, profiles);
    assert_eq!(session.actions.len(), 1);
    assert_eq!(
        session.actions[0].1,
        OperatorAction::SetMonitoringProfile {
            camera_ids: vec![1, 2],
            monitoring_profile_id: 500
        }
    );
}
