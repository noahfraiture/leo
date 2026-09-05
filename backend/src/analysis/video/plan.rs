use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::recording::RecordingSegment;
use crate::session::{OperatorAction, Session};

use super::error::{Error, Result};

/// One enabled span with a fixed cadence on the session-relative timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplingPeriod {
    /// Inclusive session-relative start; this offset is sampled immediately.
    pub start: Duration,
    /// Exclusive session-relative end.
    pub end: Duration,
    /// Fixed cadence beginning at `start`.
    pub sample_every: Duration,
}

/// A camera's ordered, non-overlapping enabled periods for one completed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplingSchedule {
    /// Camera whose participation and interval events produced this schedule.
    pub camera_id: u32,
    /// Ordered enabled spans; disabled time is omitted as gaps.
    pub periods: Vec<SamplingPeriod>,
}

impl SamplingSchedule {
    /// Replays one camera's session actions into normalized enabled periods.
    pub fn from_session(session: &Session, camera_id: u32) -> Result<Self> {
        let camera = session
            .cameras
            .iter()
            .find(|camera| camera.id == camera_id)
            .ok_or(Error::UnknownCamera { camera_id })?;
        let mut profile_id = camera.initial_monitoring_profile_id;
        let cadence = |id| {
            session
                .monitoring_profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| Duration::from_millis(profile.sample_every_ms))
                .filter(|interval| !interval.is_zero())
                .ok_or(Error::InvalidSamplingInterval { camera_id })
        };
        let mut enabled = camera.enabled;
        let mut sample_every = cadence(profile_id)?;
        let mut period_start = enabled.then_some(Duration::ZERO);
        let mut periods = Vec::new();

        for actions in session.actions.chunk_by(|left, right| left.0 == right.0) {
            let offset = actions[0].0;
            let previous_enabled = enabled;
            let previous_profile_id = profile_id;

            for (_, action) in actions {
                match action {
                    OperatorAction::SetCameraParticipation {
                        camera_id: action_camera_id,
                        enabled: next_enabled,
                    } if *action_camera_id == camera_id && *next_enabled != enabled => {
                        if offset > session.end_offset {
                            return Err(Error::ActionAfterSessionEnd {
                                camera_id,
                                offset,
                                session_end: session.end_offset,
                            });
                        }
                        enabled = *next_enabled;
                    }
                    OperatorAction::SetMonitoringProfile {
                        camera_ids,
                        monitoring_profile_id,
                    } if camera_ids.contains(&camera_id) => {
                        if offset > session.end_offset {
                            return Err(Error::ActionAfterSessionEnd {
                                camera_id,
                                offset,
                                session_end: session.end_offset,
                            });
                        }
                        profile_id = *monitoring_profile_id;
                        sample_every = cadence(profile_id)?;
                    }
                    _ => {}
                }
            }

            if enabled == previous_enabled && profile_id == previous_profile_id {
                continue;
            }

            if previous_enabled
                && let Some(start) = period_start
                && start < offset
            {
                periods.push(SamplingPeriod {
                    start,
                    end: offset,
                    sample_every: cadence(previous_profile_id)?,
                });
            }
            period_start = (enabled && offset < session.end_offset).then_some(offset);
        }

        if enabled
            && let Some(start) = period_start
            && start < session.end_offset
        {
            periods.push(SamplingPeriod {
                start,
                end: session.end_offset,
                sample_every,
            });
        }

        Ok(Self { camera_id, periods })
    }

    /// Generates ordered, unique session offsets from all normalized periods.
    pub fn sample_offsets(&self) -> Result<Vec<Duration>> {
        let mut offsets = Vec::new();
        let mut previous_end = None;

        for period in &self.periods {
            if period.start >= period.end || period.sample_every.is_zero() {
                return Err(Error::InvalidSamplingPeriod {
                    camera_id: self.camera_id,
                    start: period.start,
                    end: period.end,
                    sample_every: period.sample_every,
                });
            }
            if let Some(previous_end) = previous_end
                && previous_end > period.start
            {
                return Err(Error::UnorderedSamplingPeriods {
                    camera_id: self.camera_id,
                    previous_end,
                    start: period.start,
                });
            }

            let mut offset = period.start;
            loop {
                offsets.push(offset);
                let remaining = period.end - offset;
                if period.sample_every >= remaining {
                    break;
                }
                offset += period.sample_every;
            }
            previous_end = Some(period.end);
        }

        Ok(offsets)
    }
}

/// A recoverable physical recording gap on one camera's session timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisWarning {
    RecordingGap {
        camera_id: u32,
        start_offset_ms: u64,
        end_offset_ms: u64,
    },
}

/// Derives physical recording gaps for every session camera, independent of participation.
pub fn recording_gap_warnings(
    session: &Session,
    segments: &[RecordingSegment],
) -> Result<Vec<AnalysisWarning>> {
    let session_duration_ms =
        u64::try_from(session.end_offset.as_millis()).map_err(|_| Error::SessionEndUtcOverflow)?;
    let session_end_utc_ms = session
        .start_utc_ms
        .checked_add(i64::try_from(session_duration_ms).map_err(|_| Error::SessionEndUtcOverflow)?)
        .ok_or(Error::SessionEndUtcOverflow)?;
    let mut warnings = Vec::new();

    for camera in &session.cameras {
        let mut coverage = segments
            .iter()
            .filter(|segment| segment.camera_id == camera.id)
            .filter_map(|segment| {
                let start = segment.start_utc_ms.max(session.start_utc_ms);
                let end = segment.end_utc_ms.min(session_end_utc_ms);
                (start < end).then_some((start, end))
            })
            .collect::<Vec<_>>();
        coverage.sort_unstable();

        let mut cursor = session.start_utc_ms;
        for (start, end) in coverage {
            if cursor < start {
                push_recording_gap(session, camera.id, cursor, start, &mut warnings);
            }
            cursor = cursor.max(end);
        }
        if cursor < session_end_utc_ms {
            push_recording_gap(
                session,
                camera.id,
                cursor,
                session_end_utc_ms,
                &mut warnings,
            );
        }
    }

    warnings.sort_by_key(|warning| match warning {
        AnalysisWarning::RecordingGap {
            camera_id,
            start_offset_ms,
            end_offset_ms,
        } => (*camera_id, *start_offset_ms, *end_offset_ms),
    });
    Ok(warnings)
}

fn push_recording_gap(
    session: &Session,
    camera_id: u32,
    start_utc_ms: i64,
    end_utc_ms: i64,
    warnings: &mut Vec<AnalysisWarning>,
) {
    let start_offset_ms = u64::try_from(start_utc_ms - session.start_utc_ms)
        .expect("clipped gap start is on the session timeline");
    let end_offset_ms = u64::try_from(end_utc_ms - session.start_utc_ms)
        .expect("clipped gap end is on the session timeline");
    tracing::warn!(
        session_id = %session.id,
        camera_id,
        start_offset_ms,
        end_offset_ms,
        "physical recording gap"
    );
    warnings.push(AnalysisWarning::RecordingGap {
        camera_id,
        start_offset_ms,
        end_offset_ms,
    });
}

/// One planned sample tied to a local segment and both session and recording timelines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Camera selected at this session offset.
    pub camera_id: u32,
    /// Inclusive UTC start of the local segment containing this sample.
    pub segment_start_utc_ms: i64,
    /// Exclusive UTC end of the local segment containing this sample.
    pub segment_end_utc_ms: i64,
    /// Zero-based position across the camera's complete session sample sequence.
    pub sample_index: usize,
    /// Position on the shared session-relative timeline.
    pub session_offset: Duration,
    /// Position from the matched segment's inclusive UTC start.
    pub recording_offset: Duration,
    /// Finalized local segment used for direct extraction.
    pub path: PathBuf,
}

/// One camera's ordered planned samples across the recording segments that cover the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleSequence {
    /// Camera represented by every planned frame in the sequence.
    pub camera_id: u32,
    /// Samples ordered across the complete software session timeline.
    pub frames: Vec<Frame>,
}

impl SampleSequence {
    /// Matches each planned sample to zero or one local segment without decoding media.
    pub fn from_segments(
        session_start_utc_ms: i64,
        schedule: &SamplingSchedule,
        segments: &[RecordingSegment],
    ) -> Result<Self> {
        let sample_offsets = schedule.sample_offsets()?;
        let mut frames = Vec::with_capacity(sample_offsets.len());

        for (sample_index, session_offset) in sample_offsets.into_iter().enumerate() {
            let offset_ms = i64::try_from(session_offset.as_millis()).map_err(|_| {
                Error::UtcTimestampOverflow {
                    camera_id: schedule.camera_id,
                    session_offset,
                }
            })?;
            let sample_utc_ms =
                session_start_utc_ms
                    .checked_add(offset_ms)
                    .ok_or(Error::UtcTimestampOverflow {
                        camera_id: schedule.camera_id,
                        session_offset,
                    })?;
            let mut matching_segments = segments.iter().filter(|segment| {
                segment.camera_id == schedule.camera_id
                    && segment.start_utc_ms <= sample_utc_ms
                    && sample_utc_ms < segment.end_utc_ms
            });
            let Some(segment) = matching_segments.next() else {
                continue;
            };
            if matching_segments.next().is_some() {
                return Err(Error::OverlappingRecordings {
                    camera_id: schedule.camera_id,
                    session_offset,
                });
            }
            let recording_offset_ms = sample_utc_ms
                .checked_sub(segment.start_utc_ms)
                .and_then(|offset| u64::try_from(offset).ok())
                .ok_or(Error::UtcTimestampOverflow {
                    camera_id: schedule.camera_id,
                    session_offset,
                })?;

            frames.push(Frame {
                camera_id: schedule.camera_id,
                segment_start_utc_ms: segment.start_utc_ms,
                segment_end_utc_ms: segment.end_utc_ms,
                sample_index,
                session_offset,
                recording_offset: Duration::from_millis(recording_offset_ms),
                path: segment.path.clone(),
            });
        }

        Ok(Self {
            camera_id: schedule.camera_id,
            frames,
        })
    }
}

/// All available camera samples at one session-relative offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSet {
    /// Position shared by every frame in this set on the session timeline.
    pub session_offset: Duration,
    /// Frames ordered by camera ID.
    pub frames: Vec<Frame>,
}

impl FrameSet {
    /// Merges ordered camera sequences by session offset and camera ID.
    pub fn from_sequences(sequences: Vec<SampleSequence>) -> Result<Vec<Self>> {
        for sequence in &sequences {
            for frames in sequence.frames.windows(2) {
                if frames[0].session_offset == frames[1].session_offset {
                    return Err(Error::DuplicateCameraFrame {
                        camera_id: sequence.camera_id,
                        session_offset: frames[0].session_offset,
                    });
                }
                if frames[0].session_offset > frames[1].session_offset {
                    return Err(Error::UnorderedSequence {
                        camera_id: sequence.camera_id,
                        previous: frames[0].session_offset,
                        current: frames[1].session_offset,
                    });
                }
            }
        }

        let mut iterators = sequences
            .into_iter()
            .map(|sequence| sequence.frames.into_iter().peekable())
            .collect::<Vec<_>>();
        let mut frame_sets = Vec::new();

        loop {
            let Some(session_offset) = iterators
                .iter_mut()
                .filter_map(|frames| frames.peek().map(|frame| frame.session_offset))
                .min()
            else {
                return Ok(frame_sets);
            };
            let mut frames = iterators
                .iter_mut()
                .filter_map(|frames| {
                    if frames
                        .peek()
                        .is_some_and(|frame| frame.session_offset == session_offset)
                    {
                        frames.next()
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            frames.sort_by_key(|frame| frame.camera_id);

            if let Some(duplicate) = frames
                .windows(2)
                .find(|frames| frames[0].camera_id == frames[1].camera_id)
            {
                return Err(Error::DuplicateCameraFrame {
                    camera_id: duplicate[0].camera_id,
                    session_offset,
                });
            }

            frame_sets.push(Self {
                session_offset,
                frames,
            });
        }
    }
}

#[cfg(test)]
#[path = "tests/plan.rs"]
mod tests;
