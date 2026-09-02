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
#[path = "tests/event_log.rs"]
mod tests;
