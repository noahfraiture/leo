use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::recording::RecordingSegment;
use crate::session::{OperatorAction, Session};

use super::error::{Error, Result};

/// One enabled span with a fixed cadence on the session-relative timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::analysis) struct SamplingPeriod {
    /// Inclusive session-relative start; this offset is sampled immediately.
    pub(in crate::analysis) start: Duration,
    /// Exclusive session-relative end.
    pub(in crate::analysis) end: Duration,
    /// Fixed cadence beginning at `start`.
    pub(in crate::analysis) sample_every: Duration,
}

/// A camera's ordered, non-overlapping enabled periods for one completed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::analysis) struct SamplingSchedule {
    /// Camera whose participation and interval events produced this schedule.
    pub(in crate::analysis) camera_id: u32,
    /// Ordered enabled spans; disabled time is omitted as gaps.
    pub(in crate::analysis) periods: Vec<SamplingPeriod>,
}

impl SamplingSchedule {
    /// Replays one camera's session actions into normalized enabled periods.
    pub(in crate::analysis) fn from_session(session: &Session, camera_id: u32) -> Result<Self> {
        let camera = session
            .cameras
            .iter()
            .find(|camera| camera.id == camera_id)
            .ok_or(Error::UnknownCamera { camera_id })?;
        if camera.sample_every.is_zero() {
            return Err(Error::InvalidSamplingInterval { camera_id });
        }

        let mut enabled = camera.enabled;
        let mut sample_every = camera.sample_every;
        let mut period_start = enabled.then_some(Duration::ZERO);
        let mut periods = Vec::new();

        for actions in session.actions.chunk_by(|left, right| left.0 == right.0) {
            let offset = actions[0].0;
            let previous_enabled = enabled;
            let previous_sample_every = sample_every;

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
                    OperatorAction::SetSamplingInterval {
                        camera_id: action_camera_id,
                        sample_every: next_sample_every,
                    } if *action_camera_id == camera_id => {
                        if next_sample_every.is_zero() {
                            return Err(Error::InvalidSamplingInterval { camera_id });
                        }
                        if *next_sample_every != sample_every {
                            if offset > session.end_offset {
                                return Err(Error::ActionAfterSessionEnd {
                                    camera_id,
                                    offset,
                                    session_end: session.end_offset,
                                });
                            }
                            sample_every = *next_sample_every;
                        }
                    }
                    _ => {}
                }
            }

            if enabled == previous_enabled && sample_every == previous_sample_every {
                continue;
            }

            if previous_enabled
                && let Some(start) = period_start
                && start < offset
            {
                periods.push(SamplingPeriod {
                    start,
                    end: offset,
                    sample_every: previous_sample_every,
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
    pub(in crate::analysis) fn sample_offsets(&self) -> Result<Vec<Duration>> {
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
pub(in crate::analysis) struct SampleSequence {
    /// Camera represented by every planned frame in the sequence.
    pub(in crate::analysis) camera_id: u32,
    /// Samples ordered across the complete software session timeline.
    pub(in crate::analysis) frames: Vec<Frame>,
}

impl SampleSequence {
    /// Matches each planned sample to zero or one local segment without decoding media.
    pub(in crate::analysis) fn from_segments(
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
pub(in crate::analysis) struct FrameSet {
    /// Position shared by every frame in this set on the session timeline.
    pub(in crate::analysis) session_offset: Duration,
    /// Frames ordered by camera ID.
    pub(in crate::analysis) frames: Vec<Frame>,
}

impl FrameSet {
    /// Merges ordered camera sequences by session offset and camera ID.
    pub(in crate::analysis) fn from_sequences(sequences: Vec<SampleSequence>) -> Result<Vec<Self>> {
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
mod tests {
    use std::{path::PathBuf, time::Duration};

    use uuid::Uuid;

    use crate::{
        recording::RecordingSegment,
        session::{OperatorAction, Session, SessionCamera},
    };

    use super::{
        AnalysisWarning, Frame, FrameSet, SampleSequence, SamplingPeriod, SamplingSchedule,
        recording_gap_warnings,
    };

    const SESSION_START_UTC_MS: i64 = 1_000_000;

    fn session(
        enabled: bool,
        sample_every: Duration,
        end: Duration,
        actions: Vec<(Duration, OperatorAction)>,
    ) -> Session {
        Session {
            id: Uuid::nil(),
            start_utc_ms: SESSION_START_UTC_MS,
            end_offset: end,
            cameras: vec![SessionCamera {
                id: 1,
                name: "Front".into(),
                enabled,
                sample_every,
            }],
            actions,
        }
    }

    fn participation(offset_secs: u64, enabled: bool) -> (Duration, OperatorAction) {
        (
            Duration::from_secs(offset_secs),
            OperatorAction::SetCameraParticipation {
                camera_id: 1,
                enabled,
            },
        )
    }

    fn interval(offset_secs: u64, sample_every_secs: u64) -> (Duration, OperatorAction) {
        (
            Duration::from_secs(offset_secs),
            OperatorAction::SetSamplingInterval {
                camera_id: 1,
                sample_every: Duration::from_secs(sample_every_secs),
            },
        )
    }

    fn sample_offsets(session: &Session) -> Vec<Duration> {
        SamplingSchedule::from_session(session, 1)
            .expect("schedule should be built")
            .sample_offsets()
            .expect("sample offsets should be generated")
    }

    fn seconds(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_secs).collect()
    }

    fn segment(camera_id: u32, start_offset_ms: i64, end_offset_ms: i64) -> RecordingSegment {
        RecordingSegment {
            camera_id,
            start_utc_ms: SESSION_START_UTC_MS.checked_add(start_offset_ms).unwrap(),
            end_utc_ms: SESSION_START_UTC_MS.checked_add(end_offset_ms).unwrap(),
            path: PathBuf::from(format!("camera-{camera_id}-{start_offset_ms}.mkv")),
        }
    }

    #[test]
    fn operator_changes_define_the_sampling_schedule() {
        struct Case {
            name: &'static str,
            initially_enabled: bool,
            sample_every_secs: u64,
            actions: Vec<(Duration, OperatorAction)>,
            expected_secs: &'static [u64],
        }

        let cases = [
            Case {
                name: "initially enabled",
                initially_enabled: true,
                sample_every_secs: 3,
                actions: vec![],
                expected_secs: &[0, 3, 6, 9],
            },
            Case {
                name: "initially disabled",
                initially_enabled: false,
                sample_every_secs: 3,
                actions: vec![participation(4, true)],
                expected_secs: &[4, 7],
            },
            Case {
                name: "disabled on a scheduled sample",
                initially_enabled: true,
                sample_every_secs: 3,
                actions: vec![participation(6, false)],
                expected_secs: &[0, 3],
            },
            Case {
                name: "re-enabled with a fresh cadence",
                initially_enabled: true,
                sample_every_secs: 3,
                actions: vec![participation(2, false), participation(5, true)],
                expected_secs: &[0, 5, 8],
            },
            Case {
                name: "cadence changed",
                initially_enabled: true,
                sample_every_secs: 3,
                actions: vec![interval(4, 2)],
                expected_secs: &[0, 3, 4, 6, 8],
            },
            Case {
                name: "same-offset changes applied together",
                initially_enabled: true,
                sample_every_secs: 5,
                actions: vec![
                    participation(5, false),
                    interval(5, 2),
                    participation(5, true),
                ],
                expected_secs: &[0, 5, 7, 9],
            },
            Case {
                name: "repeated values are no-ops",
                initially_enabled: true,
                sample_every_secs: 4,
                actions: vec![participation(3, true), interval(5, 4)],
                expected_secs: &[0, 4, 8],
            },
            Case {
                name: "session end is exclusive",
                initially_enabled: true,
                sample_every_secs: 5,
                actions: vec![],
                expected_secs: &[0, 5],
            },
        ];

        for case in cases {
            let session = session(
                case.initially_enabled,
                Duration::from_secs(case.sample_every_secs),
                Duration::from_secs(10),
                case.actions,
            );

            assert_eq!(
                sample_offsets(&session),
                seconds(case.expected_secs),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn periods_are_normalized_with_disabled_time_as_a_gap() {
        let session = session(
            true,
            Duration::from_secs(2),
            Duration::from_secs(12),
            vec![
                interval(3, 1),
                participation(6, false),
                participation(8, true),
            ],
        );

        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");

        assert_eq!(
            schedule.periods,
            vec![
                SamplingPeriod {
                    start: Duration::ZERO,
                    end: Duration::from_secs(3),
                    sample_every: Duration::from_secs(2),
                },
                SamplingPeriod {
                    start: Duration::from_secs(3),
                    end: Duration::from_secs(6),
                    sample_every: Duration::from_secs(1),
                },
                SamplingPeriod {
                    start: Duration::from_secs(8),
                    end: Duration::from_secs(12),
                    sample_every: Duration::from_secs(1),
                },
            ]
        );
        assert!(
            schedule
                .periods
                .iter()
                .all(|period| period.start < period.end && !period.sample_every.is_zero())
        );
        assert!(
            schedule
                .periods
                .windows(2)
                .all(|periods| periods[0].end <= periods[1].start)
        );
    }

    #[test]
    fn zero_sampling_intervals_are_rejected() {
        let session = session(true, Duration::ZERO, Duration::from_secs(10), vec![]);

        assert!(SamplingSchedule::from_session(&session, 1).is_err());
    }

    #[test]
    fn sequences_keep_global_indices_and_recording_offsets_across_segments() {
        let session = session(true, Duration::from_secs(2), Duration::from_secs(8), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let segments = vec![segment(1, -1_000, 4_000), segment(1, 4_000, 8_000)];

        let sequence = SampleSequence::from_segments(SESSION_START_UTC_MS, &schedule, &segments)
            .expect("segments should cover every sample");

        assert_eq!(
            sequence,
            SampleSequence {
                camera_id: 1,
                frames: vec![
                    Frame {
                        camera_id: 1,
                        segment_start_utc_ms: SESSION_START_UTC_MS - 1_000,
                        segment_end_utc_ms: SESSION_START_UTC_MS + 4_000,
                        sample_index: 0,
                        session_offset: Duration::ZERO,
                        recording_offset: Duration::from_secs(1),
                        path: segments[0].path.clone(),
                    },
                    Frame {
                        camera_id: 1,
                        segment_start_utc_ms: SESSION_START_UTC_MS - 1_000,
                        segment_end_utc_ms: SESSION_START_UTC_MS + 4_000,
                        sample_index: 1,
                        session_offset: Duration::from_secs(2),
                        recording_offset: Duration::from_secs(3),
                        path: segments[0].path.clone(),
                    },
                    Frame {
                        camera_id: 1,
                        segment_start_utc_ms: SESSION_START_UTC_MS + 4_000,
                        segment_end_utc_ms: SESSION_START_UTC_MS + 8_000,
                        sample_index: 2,
                        session_offset: Duration::from_secs(4),
                        recording_offset: Duration::ZERO,
                        path: segments[1].path.clone(),
                    },
                    Frame {
                        camera_id: 1,
                        segment_start_utc_ms: SESSION_START_UTC_MS + 4_000,
                        segment_end_utc_ms: SESSION_START_UTC_MS + 8_000,
                        sample_index: 3,
                        session_offset: Duration::from_secs(6),
                        recording_offset: Duration::from_secs(2),
                        path: segments[1].path.clone(),
                    },
                ],
            }
        );
    }

    #[test]
    fn sequence_skips_missing_samples_and_resumes_after_the_gap() {
        let session = session(true, Duration::from_secs(1), Duration::from_secs(4), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let segments = vec![segment(1, 0, 1_000), segment(1, 3_000, 4_000)];

        let sequence = SampleSequence::from_segments(SESSION_START_UTC_MS, &schedule, &segments)
            .expect("missing coverage should skip only its samples");

        assert_eq!(
            sequence
                .frames
                .iter()
                .map(|frame| (frame.sample_index, frame.session_offset))
                .collect::<Vec<_>>(),
            vec![(0, Duration::ZERO), (3, Duration::from_secs(3))]
        );
        assert_eq!(sequence.frames[0].path, segments[0].path);
        assert_eq!(sequence.frames[1].path, segments[1].path);
    }

    #[test]
    fn sequence_still_rejects_overlapping_segments() {
        let session = session(true, Duration::from_secs(2), Duration::from_secs(5), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let segments = vec![segment(1, 0, 3_000), segment(1, 2_000, 5_000)];

        let error = SampleSequence::from_segments(SESSION_START_UTC_MS, &schedule, &segments)
            .expect_err("the sample at two seconds has two recordings");

        assert!(error.to_string().contains("multiple recordings"));
    }

    fn frame(camera_id: u32, sample_index: usize, offset_secs: u64) -> Frame {
        let offset = Duration::from_secs(offset_secs);
        Frame {
            camera_id,
            segment_start_utc_ms: SESSION_START_UTC_MS,
            segment_end_utc_ms: SESSION_START_UTC_MS + 10_000,
            sample_index,
            session_offset: offset,
            recording_offset: offset,
            path: PathBuf::from(format!("camera-{camera_id}.mkv")),
        }
    }

    fn sequence(camera_id: u32, offsets: &[u64]) -> SampleSequence {
        SampleSequence {
            camera_id,
            frames: offsets
                .iter()
                .enumerate()
                .map(|(sample_index, offset)| frame(camera_id, sample_index, *offset))
                .collect(),
        }
    }

    #[test]
    fn frame_sets_merge_mixed_intervals_into_partial_sets() {
        let frame_sets =
            FrameSet::from_sequences(vec![sequence(1, &[0, 4, 8]), sequence(2, &[0, 2, 4, 6, 8])])
                .expect("ordered sequences should merge");

        assert_eq!(
            frame_sets
                .iter()
                .map(|frame_set| frame_set.session_offset)
                .collect::<Vec<_>>(),
            seconds(&[0, 2, 4, 6, 8])
        );
        assert_eq!(
            frame_sets
                .iter()
                .map(|frame_set| {
                    frame_set
                        .frames
                        .iter()
                        .map(|frame| frame.camera_id)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            vec![vec![1, 2], vec![2], vec![1, 2], vec![2], vec![1, 2]]
        );
    }

    #[test]
    fn missing_camera_frame_keeps_the_other_camera_frame_set() {
        let schedules = [
            SamplingSchedule {
                camera_id: 1,
                periods: vec![SamplingPeriod {
                    start: Duration::ZERO,
                    end: Duration::from_secs(2),
                    sample_every: Duration::from_secs(1),
                }],
            },
            SamplingSchedule {
                camera_id: 2,
                periods: vec![SamplingPeriod {
                    start: Duration::ZERO,
                    end: Duration::from_secs(2),
                    sample_every: Duration::from_secs(1),
                }],
            },
        ];
        let segments = vec![segment(1, 0, 1_000), segment(2, 0, 2_000)];
        let sequences = schedules
            .iter()
            .map(|schedule| {
                SampleSequence::from_segments(SESSION_START_UTC_MS, schedule, &segments)
            })
            .collect::<Result<Vec<_>, _>>()
            .expect("missing coverage should leave partial sequences");

        let frame_sets = FrameSet::from_sequences(sequences).expect("partial sets should merge");

        assert_eq!(
            frame_sets
                .iter()
                .map(|frame_set| {
                    (
                        frame_set.session_offset,
                        frame_set
                            .frames
                            .iter()
                            .map(|frame| frame.camera_id)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (Duration::ZERO, vec![1, 2]),
                (Duration::from_secs(1), vec![2]),
            ]
        );
    }

    #[test]
    fn gaps_before_between_and_after_segments_are_coalesced() {
        let session = session(
            true,
            Duration::from_secs(1),
            Duration::from_secs(10),
            vec![],
        );
        let segments = vec![
            segment(1, 6_000, 8_000),
            segment(1, 2_000, 4_000),
            segment(1, 1_000, 2_000),
        ];

        let warnings = recording_gap_warnings(&session, &segments)
            .expect("valid session bounds should produce warnings");

        assert_eq!(
            warnings,
            vec![
                AnalysisWarning::RecordingGap {
                    camera_id: 1,
                    start_offset_ms: 0,
                    end_offset_ms: 1_000,
                },
                AnalysisWarning::RecordingGap {
                    camera_id: 1,
                    start_offset_ms: 4_000,
                    end_offset_ms: 6_000,
                },
                AnalysisWarning::RecordingGap {
                    camera_id: 1,
                    start_offset_ms: 8_000,
                    end_offset_ms: 10_000,
                },
            ]
        );
    }

    #[test]
    fn disabled_participation_does_not_hide_a_physical_recording_gap() {
        let session = session(
            true,
            Duration::from_secs(1),
            Duration::from_secs(6),
            vec![participation(1, false), participation(5, true)],
        );
        let segments = vec![segment(1, 0, 2_000), segment(1, 4_000, 6_000)];

        let warnings = recording_gap_warnings(&session, &segments)
            .expect("participation should not affect physical coverage");

        assert_eq!(
            warnings,
            vec![AnalysisWarning::RecordingGap {
                camera_id: 1,
                start_offset_ms: 2_000,
                end_offset_ms: 4_000,
            }]
        );
    }

    #[test]
    fn camera_without_segments_gets_one_full_session_gap() {
        let mut session = session(true, Duration::from_secs(1), Duration::from_secs(5), vec![]);
        session.cameras.push(SessionCamera {
            id: 2,
            name: "Side".into(),
            enabled: false,
            sample_every: Duration::from_secs(1),
        });

        let warnings = recording_gap_warnings(&session, &[segment(1, 0, 5_000)])
            .expect("every session camera should be checked");

        assert_eq!(
            warnings,
            vec![AnalysisWarning::RecordingGap {
                camera_id: 2,
                start_offset_ms: 0,
                end_offset_ms: 5_000,
            }]
        );
    }

    #[test]
    fn frame_sets_reject_duplicate_camera_frames_at_one_offset() {
        let duplicate_inputs = [
            vec![sequence(1, &[0, 0])],
            vec![sequence(1, &[0]), sequence(1, &[0])],
        ];

        for sequences in duplicate_inputs {
            let error = FrameSet::from_sequences(sequences)
                .expect_err("one camera may contribute at most one frame per offset");
            assert!(error.to_string().contains("duplicate camera 1 frame"));
        }
    }
}
