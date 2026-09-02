//! Validates operator actions and durably appends them to one active session event log.

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

/// An action durably appended to the active session's `events.jsonl` timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorAction {
    /// Includes or excludes a camera from subsequent software sampling.
    SetCameraParticipation { camera_id: u32, enabled: bool },
    /// Changes a camera's software sampling cadence.
    SetSamplingInterval {
        camera_id: u32,
        sample_every: Duration,
    },
    /// Ends the persisted session timeline; recorder shutdown remains the caller's responsibility.
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

    /// Validates and durably records one action before returning.
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
#[path = "tests/controller.rs"]
mod tests;
