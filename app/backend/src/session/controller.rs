use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use uuid::Uuid;

use super::{
    error::{Error, Result},
    event_log::{
        SCHEMA_VERSION, SessionAction, SessionCamera, SessionEvent, camera_ids, duration_to_millis,
    },
};

/// Software-session actions accepted from future UI and internal callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorAction {
    /// Includes or excludes a camera from subsequent software sampling.
    SetCameraParticipation { camera_id: u32, enabled: bool },
    /// Changes a camera's software sampling cadence.
    SetSamplingInterval {
        camera_id: u32,
        sample_every: Duration,
    },
    /// Ends the software session timeline without affecting physical recording.
    EndSession,
}

/// The single backend entry point that validates, routes, and serializes session actions.
#[derive(Debug)]
pub struct SessionController {
    log: SessionLog,
}

impl SessionController {
    /// Creates a new event log and durably writes its session-start event.
    pub fn create(events_path: PathBuf, cameras: Vec<SessionCamera>) -> Result<Self> {
        let camera_ids = camera_ids(&cameras)?;
        let action = SessionAction::SessionStarted { cameras };

        Ok(Self {
            log: SessionLog::create(events_path, camera_ids, action)?,
        })
    }

    /// Validates and durably records one operator action.
    pub fn apply(&mut self, action: OperatorAction) -> Result<()> {
        if self.log.ended {
            return Err(Error::SessionEnded);
        }

        match action {
            OperatorAction::SetCameraParticipation { camera_id, enabled } => {
                self.log.require_camera(camera_id)?;
                self.log
                    .append(SessionAction::CameraParticipationChanged { camera_id, enabled })
            }
            OperatorAction::SetSamplingInterval {
                camera_id,
                sample_every,
            } => {
                self.log.require_camera(camera_id)?;
                let sample_every_ms = duration_to_millis(camera_id, sample_every)?;
                self.log.append(SessionAction::SamplingIntervalChanged {
                    camera_id,
                    sample_every_ms,
                })
            }
            OperatorAction::EndSession => {
                self.log.append(SessionAction::SessionEnded)?;
                self.log.ended = true;
                Ok(())
            }
        }
    }

    /// Returns monotonic elapsed time since the session-start event was written.
    pub fn elapsed(&self) -> Duration {
        self.log.started_at.elapsed()
    }
}

#[derive(Debug)]
struct SessionLog {
    file: File,
    session_id: Uuid,
    started_at: Instant,
    camera_ids: HashSet<u32>,
    next_sequence: u64,
    ended: bool,
}

impl SessionLog {
    fn create(
        events_path: PathBuf,
        camera_ids: HashSet<u32>,
        start_action: SessionAction,
    ) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(events_path)?;
        let mut log = Self {
            file,
            session_id: Uuid::new_v4(),
            started_at: Instant::now(),
            camera_ids,
            next_sequence: 0,
            ended: false,
        };
        log.write(start_action, 0)?;

        Ok(log)
    }

    fn require_camera(&self, camera_id: u32) -> Result<()> {
        if self.camera_ids.contains(&camera_id) {
            Ok(())
        } else {
            Err(Error::UnknownCamera { camera_id })
        }
    }

    fn append(&mut self, action: SessionAction) -> Result<()> {
        let session_offset_ms = u64::try_from(self.started_at.elapsed().as_millis())
            .map_err(|_| Error::SessionOffsetOverflow)?;
        self.write(action, session_offset_ms)
    }

    fn write(&mut self, action: SessionAction, session_offset_ms: u64) -> Result<()> {
        let event = SessionEvent {
            schema_version: SCHEMA_VERSION,
            sequence: self.next_sequence,
            session_id: self.session_id,
            utc_ms: utc_now_ms()?,
            session_offset_ms,
            action,
        };

        serde_json::to_writer(&mut self.file, &event).map_err(Error::Serialize)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file.sync_data()?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(Error::SequenceOverflow)?;

        Ok(())
    }
}

fn utc_now_ms() -> Result<i64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    i64::try_from(millis).map_err(|_| Error::UtcTimestampOverflow)
}

#[cfg(test)]
mod tests {
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
    fn create_rejects_duplicate_cameras_before_creating_a_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("events.jsonl");

        let error = SessionController::create(
            path.clone(),
            vec![
                camera(1, Duration::from_secs(1)),
                camera(1, Duration::from_secs(2)),
            ],
        )
        .expect_err("camera IDs should be unique");

        assert!(error.to_string().contains("duplicate camera 1"));
        assert!(!path.exists());
    }

    #[test]
    fn create_rejects_zero_initial_intervals_before_creating_a_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("events.jsonl");

        let error = SessionController::create(path.clone(), vec![camera(1, Duration::ZERO)])
            .expect_err("sampling intervals should be positive");

        assert!(error.to_string().contains("sampling interval"));
        assert!(!path.exists());
    }

    #[test]
    fn session_controller_rejects_empty_and_zero_id_camera_lists() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        for (name, cameras, expected) in [
            ("empty.jsonl", vec![], "at least one camera"),
            (
                "zero-id.jsonl",
                vec![camera(0, Duration::from_secs(1))],
                "camera ID must be non-zero",
            ),
        ] {
            let path = directory.path().join(name);

            let error = SessionController::create(path.clone(), cameras)
                .expect_err("invalid cameras should be rejected");

            assert!(error.to_string().contains(expected));
            assert!(!path.exists());
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
}
