use std::{fs, path::Path, time::Duration};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{OperatorAction, Session, SessionCamera};

const SESSION_ID: &str = "5a660250-36fc-4c2b-93fa-b04247bdad20";
const OTHER_SESSION_ID: &str = "74690993-6174-4312-9d72-fb5f7127d9d4";
const START_UTC_MS: i64 = 1_786_204_800_000;

fn camera(camera_id: u32, sample_every_ms: u64) -> Value {
    json!({
        "camera_id": camera_id,
        "name": format!("Camera {camera_id}"),
        "enabled": true,
        "initial_monitoring_profile_id": sample_every_ms
    })
}

fn event(sequence: u64, session_id: &str, offset_ms: u64, action: Value) -> Value {
    json!({
        "schema_version": 2,
        "sequence": sequence,
        "session_id": session_id,
        "utc_ms": START_UTC_MS + i64::try_from(offset_ms).unwrap(),
        "session_offset_ms": offset_ms,
        "action": action
    })
}

fn started(cameras: Vec<Value>) -> Value {
    event(
        0,
        SESSION_ID,
        0,
        json!({"type": "session_started", "monitoring_profiles": crate::tests::monitoring_profiles(), "cameras": cameras}),
    )
}

fn ended(sequence: u64, session_id: &str, offset_ms: u64) -> Value {
    event(
        sequence,
        session_id,
        offset_ms,
        json!({"type": "session_ended"}),
    )
}

fn write_events(path: &Path, events: &[Value]) {
    let mut contents = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    contents.push('\n');
    fs::write(path, contents).expect("test events should be written");
}

fn load_error(events: &[Value]) -> String {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    write_events(&path, events);
    Session::load(&path)
        .expect_err("invalid events should be rejected")
        .to_string()
}

#[test]
fn loads_the_representative_log_with_its_utc_anchor_and_exclusive_end() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    write_events(
        &path,
        &[
            event(
                0,
                SESSION_ID,
                0,
                json!({
                    "type": "session_started", "monitoring_profiles": crate::tests::monitoring_profiles(),
                    "cameras": [
                        {"camera_id": 1, "name": "Front", "enabled": true, "initial_monitoring_profile_id": 5_000},
                        {"camera_id": 2, "name": "Side", "enabled": true, "initial_monitoring_profile_id": 2_000}
                    ]
                }),
            ),
            event(
                1,
                SESSION_ID,
                10_000,
                json!({
                    "type": "camera_participation_changed",
                    "camera_id": 2,
                    "enabled": false
                }),
            ),
            event(
                2,
                SESSION_ID,
                15_000,
                json!({
                    "type": "monitoring_profile_changed", "camera_ids": [1], "monitoring_profile_id": 1_000
                }),
            ),
            ended(3, SESSION_ID, 30_000),
        ],
    );

    let session = Session::load(&path).expect("completed session should load");

    assert_eq!(session.id, Uuid::parse_str(SESSION_ID).unwrap());
    assert_eq!(session.start_utc_ms, START_UTC_MS);
    assert_eq!(session.end_offset, Duration::from_secs(30));
    assert_eq!(
        session.cameras,
        vec![
            SessionCamera {
                id: 1,
                name: "Front".into(),
                enabled: true,
                initial_monitoring_profile_id: (5 * 1000) as u32,
            },
            SessionCamera {
                id: 2,
                name: "Side".into(),
                enabled: true,
                initial_monitoring_profile_id: (2 * 1000) as u32,
            },
        ]
    );
    assert_eq!(
        session.actions,
        vec![
            (
                Duration::from_secs(10),
                OperatorAction::SetCameraParticipation {
                    camera_id: 2,
                    enabled: false,
                },
            ),
            (
                Duration::from_secs(15),
                OperatorAction::SetMonitoringProfile {
                    camera_ids: vec![1],
                    monitoring_profile_id: 1000
                },
            ),
        ]
    );
}

#[test]
fn rejects_malformed_json() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    fs::write(&path, "not json\n").expect("malformed event should be written");

    let error = Session::load(&path)
        .expect_err("malformed JSON should be rejected")
        .to_string();

    assert!(error.contains("line 1"));
    assert!(error.contains("valid JSON"));
}

#[test]
fn rejects_a_missing_final_newline() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("events.jsonl");
    let events = [started(vec![camera(1, 1_000)]), ended(1, SESSION_ID, 1_000)];
    let contents = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, contents).expect("unterminated events should be written");

    let error = Session::load(&path)
        .expect_err("the final event should be newline terminated")
        .to_string();

    assert!(error.contains("final newline"));
}

#[test]
fn rejects_unsupported_schema_versions() {
    let mut events = vec![started(vec![camera(1, 1_000)]), ended(1, SESSION_ID, 1_000)];
    events[1]["schema_version"] = json!(1);

    let error = load_error(&events);

    assert!(error.contains("schema version 1"));
}

#[test]
fn rejects_unknown_fields_on_events() {
    let mut events = vec![started(vec![camera(1, 1_000)]), ended(1, SESSION_ID, 1_000)];
    events[1]["unexpected"] = json!(true);

    let error = load_error(&events);

    assert!(error.contains("valid JSON"));
}

#[test]
fn rejects_unknown_fields_on_actions() {
    let mut events = vec![
        started(vec![camera(1, 1_000)]),
        event(
            1,
            SESSION_ID,
            500,
            json!({
                "type": "camera_participation_changed",
                "camera_id": 1,
                "enabled": false
            }),
        ),
        ended(2, SESSION_ID, 1_000),
    ];
    events[1]["action"]["unexpected"] = json!(true);

    let error = load_error(&events);

    assert!(error.contains("valid JSON"));
}

#[test]
fn rejects_unknown_fields_on_session_cameras() {
    let mut events = vec![started(vec![camera(1, 1_000)]), ended(1, SESSION_ID, 1_000)];
    events[0]["action"]["cameras"][0]["unexpected"] = json!(true);

    let error = load_error(&events);

    assert!(error.contains("valid JSON"));
}

#[test]
fn rejects_mismatched_session_ids() {
    let events = vec![
        started(vec![camera(1, 1_000)]),
        ended(1, OTHER_SESSION_ID, 1_000),
    ];

    let error = load_error(&events);

    assert!(error.contains("session ID"));
    assert!(error.contains(OTHER_SESSION_ID));
}

#[test]
fn rejects_non_contiguous_sequences() {
    let events = vec![started(vec![camera(1, 1_000)]), ended(2, SESSION_ID, 1_000)];

    let error = load_error(&events);

    assert!(error.contains("expected sequence 1"));
    assert!(error.contains("found 2"));
}

#[test]
fn rejects_nonzero_session_start_offset() {
    let mut events = vec![started(vec![camera(1, 1_000)]), ended(1, SESSION_ID, 1_000)];
    events[0]["session_offset_ms"] = json!(1);

    let error = load_error(&events);

    assert!(error.contains("session start offset must be zero"));
}

#[test]
fn rejects_decreasing_action_offsets() {
    let events = vec![
        started(vec![camera(1, 1_000)]),
        event(
            1,
            SESSION_ID,
            500,
            json!({
                "type": "camera_participation_changed",
                "camera_id": 1,
                "enabled": false
            }),
        ),
        event(
            2,
            SESSION_ID,
            499,
            json!({
                "type": "monitoring_profile_changed", "camera_ids": [1], "monitoring_profile_id": 2_000
            }),
        ),
        ended(3, SESSION_ID, 1_000),
    ];

    let error = load_error(&events);

    assert!(error.contains("session offsets must be nondecreasing"));
    assert!(error.contains("500"));
    assert!(error.contains("499"));
}

#[test]
fn rejects_duplicate_initial_cameras() {
    let events = vec![
        started(vec![camera(1, 1_000), camera(1, 2_000)]),
        ended(1, SESSION_ID, 1_000),
    ];

    let error = load_error(&events);

    assert!(error.contains("duplicate camera 1"));
}

#[test]
fn rejects_unknown_initial_and_changed_profiles() {
    let invalid_logs = [
        vec![started(vec![camera(1, 0)]), ended(1, SESSION_ID, 1_000)],
        vec![
            started(vec![camera(1, 1_000)]),
            event(
                1,
                SESSION_ID,
                500,
                json!({
                    "type": "monitoring_profile_changed", "camera_ids": [1], "monitoring_profile_id": 0
                }),
            ),
            ended(2, SESSION_ID, 1_000),
        ],
    ];

    for events in invalid_logs {
        assert!(load_error(&events).contains("monitoring profile 0 does not exist"));
    }
}

#[test]
fn rejects_actions_for_unknown_cameras() {
    let actions = [
        json!({
            "type": "camera_participation_changed",
            "camera_id": 9,
            "enabled": false
        }),
        json!({
            "type": "monitoring_profile_changed", "camera_ids": [9], "monitoring_profile_id": 1_000
        }),
    ];

    for action in actions {
        let events = vec![
            started(vec![camera(1, 1_000)]),
            event(1, SESSION_ID, 500, action),
            ended(2, SESSION_ID, 1_000),
        ];
        assert!(load_error(&events).contains("unknown camera 9"));
    }
}

#[test]
fn rejects_actions_after_session_end() {
    let events = vec![
        started(vec![camera(1, 1_000)]),
        ended(1, SESSION_ID, 1_000),
        event(
            2,
            SESSION_ID,
            1_001,
            json!({
                "type": "camera_participation_changed",
                "camera_id": 1,
                "enabled": false
            }),
        ),
    ];

    let error = load_error(&events);

    assert!(error.contains("action after session end"));
}

#[test]
fn rejects_missing_session_end() {
    let events = vec![started(vec![camera(1, 1_000)])];

    let error = load_error(&events);

    assert!(error.contains("missing session end"));
}
