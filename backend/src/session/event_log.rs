//! Defines and strictly replays the private, versioned JSONL session event schema.

use std::{
    collections::HashSet,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    controller::OperatorAction,
    error::{Error, Result},
};

pub(super) const SCHEMA_VERSION: u8 = 1;

/// One camera's software sampling configuration at the start of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCamera {
    /// Stable camera ID shared by recording, analysis, and UI state.
    #[serde(rename = "camera_id")]
    pub id: u32,
    /// Operator-facing camera name included in analysis prompts.
    pub name: String,
    /// Whether software sampling begins enabled; physical recording is unaffected.
    pub enabled: bool,
    /// Initial interval between selected samples.
    #[serde(rename = "sample_every_ms", with = "duration_millis")]
    pub sample_every: Duration,
}

/// A completed session reconstructed from its event log.
#[derive(Debug, PartialEq, Eq)]
pub struct Session {
    /// UUID shared by the event log and analysis checkpoint.
    pub id: Uuid,
    /// UTC millisecond anchor captured by the session-start event.
    pub start_utc_ms: i64,
    /// Exclusive end on the session-relative timeline.
    pub end_offset: Duration,
    /// Camera configuration captured by the session-start event.
    pub cameras: Vec<SessionCamera>,
    /// Camera changes paired with their session-relative offsets.
    pub actions: Vec<(Duration, OperatorAction)>,
}

impl Session {
    /// Loads and validates a completed JSONL event log.
    pub fn load(events_path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(events_path)?;
        if !metadata.file_type().is_file() {
            return Err(Error::InvalidEventFile);
        }
        let mut file = File::open(events_path)?;
        if metadata.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut final_byte = [0];
            file.read_exact(&mut final_byte)?;
            if final_byte[0] != b'\n' {
                return Err(Error::MissingFinalNewline);
            }
            file.rewind()?;
        }
        let mut lines = BufReader::new(file).lines().enumerate();
        let (first_line_index, first_line) = lines.next().ok_or(Error::MissingSessionStart)?;
        let first = parse_event(first_line?, first_line_index + 1)?;
        validate_schema_and_sequence(&first, 0)?;

        let SessionEvent {
            session_id: id,
            utc_ms: start_utc_ms,
            session_offset_ms,
            action,
            ..
        } = first;
        if session_offset_ms != 0 {
            return Err(Error::NonZeroSessionStartOffset {
                actual: session_offset_ms,
            });
        }
        let cameras = match action {
            SessionAction::SessionStarted { cameras } => cameras,
            _ => return Err(Error::MissingSessionStart),
        };
        let camera_ids = camera_ids(&cameras)?;
        let mut actions = Vec::new();
        let mut end = None;
        let mut previous_offset_ms = session_offset_ms;

        for (line_index, line) in lines {
            let event = parse_event(line?, line_index + 1)?;
            let expected_sequence =
                u64::try_from(line_index).map_err(|_| Error::SequenceOverflow)?;
            validate_schema_and_sequence(&event, expected_sequence)?;
            if event.session_id != id {
                return Err(Error::SessionIdMismatch {
                    expected: id,
                    actual: event.session_id,
                });
            }
            if event.session_offset_ms < previous_offset_ms {
                return Err(Error::DecreasingSessionOffset {
                    sequence: event.sequence,
                    previous: previous_offset_ms,
                    actual: event.session_offset_ms,
                });
            }
            previous_offset_ms = event.session_offset_ms;

            if end.is_some() {
                return match event.action {
                    SessionAction::SessionEnded => Err(Error::DuplicateSessionEnd),
                    _ => Err(Error::ActionAfterSessionEnd),
                };
            }

            let offset = Duration::from_millis(event.session_offset_ms);
            match event.action {
                SessionAction::SessionStarted { .. } => {
                    return Err(Error::DuplicateSessionStart {
                        sequence: event.sequence,
                    });
                }
                SessionAction::CameraParticipationChanged { camera_id, enabled } => {
                    require_camera(&camera_ids, camera_id)?;
                    actions.push((
                        offset,
                        OperatorAction::SetCameraParticipation { camera_id, enabled },
                    ));
                }
                SessionAction::SamplingIntervalChanged {
                    camera_id,
                    sample_every_ms,
                } => {
                    require_camera(&camera_ids, camera_id)?;
                    let sample_every = Duration::from_millis(sample_every_ms);
                    duration_to_millis(camera_id, sample_every)?;
                    actions.push((
                        offset,
                        OperatorAction::SetSamplingInterval {
                            camera_id,
                            sample_every,
                        },
                    ));
                }
                SessionAction::SessionEnded => end = Some(offset),
            }
        }

        Ok(Self {
            id,
            start_utc_ms,
            end_offset: end.ok_or(Error::MissingSessionEnd)?,
            cameras,
            actions,
        })
    }
}

/// One ordered line in the private persisted JSONL session schema.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionEvent {
    /// Version of the persisted event schema.
    pub(super) schema_version: u8,
    /// Zero-based contiguous position in the event log.
    pub(super) sequence: u64,
    /// Session UUID shared by every line in the log.
    pub(super) session_id: Uuid,
    /// UTC audit timestamp in milliseconds since the Unix epoch.
    pub(super) utc_ms: i64,
    /// Deterministic position on the session timeline in milliseconds.
    pub(super) session_offset_ms: u64,
    pub(super) action: SessionAction,
}

/// Persisted session actions; participation and cadence variants affect only sampling.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SessionAction {
    SessionStarted {
        cameras: Vec<SessionCamera>,
    },
    CameraParticipationChanged {
        camera_id: u32,
        enabled: bool,
    },
    SamplingIntervalChanged {
        camera_id: u32,
        sample_every_ms: u64,
    },
    SessionEnded,
}

pub(super) fn camera_ids(cameras: &[SessionCamera]) -> Result<HashSet<u32>> {
    if cameras.is_empty() {
        return Err(Error::EmptyCameraList);
    }
    let mut camera_ids = HashSet::with_capacity(cameras.len());
    for camera in cameras {
        if camera.id == 0 {
            return Err(Error::ZeroCameraId);
        }
        duration_to_millis(camera.id, camera.sample_every)?;
        if !camera_ids.insert(camera.id) {
            return Err(Error::DuplicateCamera {
                camera_id: camera.id,
            });
        }
    }
    Ok(camera_ids)
}

pub(super) fn duration_to_millis(camera_id: u32, duration: Duration) -> Result<u64> {
    let milliseconds = u64::try_from(duration.as_millis())
        .map_err(|_| Error::InvalidSamplingInterval { camera_id })?;
    if milliseconds == 0 {
        Err(Error::InvalidSamplingInterval { camera_id })
    } else {
        Ok(milliseconds)
    }
}

mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let milliseconds = u64::try_from(duration.as_millis()).map_err(|_| {
            <S::Error as serde::ser::Error>::custom(
                "sampling interval is outside the persisted millisecond range",
            )
        })?;
        serializer.serialize_u64(milliseconds)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}

fn parse_event(line: String, line_number: usize) -> Result<SessionEvent> {
    serde_json::from_str(&line).map_err(|source| Error::Json {
        line: line_number,
        source,
    })
}

fn validate_schema_and_sequence(event: &SessionEvent, expected_sequence: u64) -> Result<()> {
    if event.schema_version != SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema {
            version: event.schema_version,
        });
    }
    if event.sequence != expected_sequence {
        return Err(Error::NonContiguousSequence {
            expected: expected_sequence,
            actual: event.sequence,
        });
    }
    Ok(())
}

fn require_camera(camera_ids: &HashSet<u32>, camera_id: u32) -> Result<()> {
    if camera_ids.contains(&camera_id) {
        Ok(())
    } else {
        Err(Error::UnknownCamera { camera_id })
    }
}

#[cfg(test)]
mod tests {
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
            "sample_every_ms": sample_every_ms
        })
    }

    fn event(sequence: u64, session_id: &str, offset_ms: u64, action: Value) -> Value {
        json!({
            "schema_version": 1,
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
            json!({"type": "session_started", "cameras": cameras}),
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
                        "type": "session_started",
                        "cameras": [
                            {"camera_id": 1, "name": "Front", "enabled": true, "sample_every_ms": 5_000},
                            {"camera_id": 2, "name": "Side", "enabled": true, "sample_every_ms": 2_000}
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
                        "type": "sampling_interval_changed",
                        "camera_id": 1,
                        "sample_every_ms": 1_000
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
                    sample_every: Duration::from_secs(5),
                },
                SessionCamera {
                    id: 2,
                    name: "Side".into(),
                    enabled: true,
                    sample_every: Duration::from_secs(2),
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
                    OperatorAction::SetSamplingInterval {
                        camera_id: 1,
                        sample_every: Duration::from_secs(1),
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
        events[1]["schema_version"] = json!(2);

        let error = load_error(&events);

        assert!(error.contains("schema version 2"));
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
                    "type": "sampling_interval_changed",
                    "camera_id": 1,
                    "sample_every_ms": 2_000
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
    fn rejects_zero_initial_and_changed_intervals() {
        let invalid_logs = [
            vec![started(vec![camera(1, 0)]), ended(1, SESSION_ID, 1_000)],
            vec![
                started(vec![camera(1, 1_000)]),
                event(
                    1,
                    SESSION_ID,
                    500,
                    json!({
                        "type": "sampling_interval_changed",
                        "camera_id": 1,
                        "sample_every_ms": 0
                    }),
                ),
                ended(2, SESSION_ID, 1_000),
            ],
        ];

        for events in invalid_logs {
            assert!(load_error(&events).contains("sampling interval"));
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
                "type": "sampling_interval_changed",
                "camera_id": 9,
                "sample_every_ms": 1_000
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
}
