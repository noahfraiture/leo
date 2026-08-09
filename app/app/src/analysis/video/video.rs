use std::time::Duration;

use crate::session::{OperatorAction, Session};

use super::error::{Error, Result};

/// One catalogued Surveillance Station segment with inclusive-start, exclusive-end UTC bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Video {
    /// Surveillance Station catalogue event ID used for downloads.
    pub(crate) recording_id: u64,
    /// Camera that produced this segment.
    pub(crate) camera_id: u32,
    /// Inclusive recording start in UTC milliseconds since the Unix epoch.
    pub(crate) start_utc_ms: i64,
    /// Exclusive recording end in UTC milliseconds since the Unix epoch.
    pub(crate) end_utc_ms: i64,
}

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
        let mut action_index = 0;

        while action_index < session.actions.len() {
            let offset = session.actions[action_index].0;
            let previous_enabled = enabled;
            let previous_sample_every = sample_every;

            while action_index < session.actions.len() && session.actions[action_index].0 == offset
            {
                match &session.actions[action_index].1 {
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
                action_index += 1;
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

/// One planned sample tied to a recording, with offsets for both session and recording timelines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::analysis) struct Frame {
    /// Camera selected at this session offset.
    pub(in crate::analysis) camera_id: u32,
    /// Catalogue segment containing the selected sample.
    pub(in crate::analysis) recording_id: u64,
    /// Zero-based position across the camera's complete session sample sequence.
    pub(in crate::analysis) sample_index: usize,
    /// Position on the shared session-relative timeline.
    pub(in crate::analysis) session_offset: Duration,
    /// Position from the matched recording's inclusive UTC start.
    pub(in crate::analysis) recording_offset: Duration,
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
    /// Matches each planned sample to exactly one covering recording segment without decoding media.
    pub(in crate::analysis) fn from_videos(
        session_start_utc_ms: i64,
        schedule: &SamplingSchedule,
        videos: &[Video],
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
            let mut matching_videos = videos.iter().filter(|video| {
                video.camera_id == schedule.camera_id
                    && video.start_utc_ms <= sample_utc_ms
                    && sample_utc_ms < video.end_utc_ms
            });
            let video = matching_videos.next().ok_or(Error::MissingRecording {
                camera_id: schedule.camera_id,
                session_offset,
            })?;
            if matching_videos.next().is_some() {
                return Err(Error::OverlappingRecordings {
                    camera_id: schedule.camera_id,
                    session_offset,
                });
            }
            let recording_offset_ms = sample_utc_ms.abs_diff(video.start_utc_ms);

            frames.push(Frame {
                camera_id: schedule.camera_id,
                recording_id: video.recording_id,
                sample_index,
                session_offset,
                recording_offset: Duration::from_millis(recording_offset_ms),
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
    use std::time::Duration;

    use uuid::Uuid;

    use crate::session::{OperatorAction, Session, SessionCamera};

    use super::{Frame, FrameSet, SampleSequence, SamplingPeriod, SamplingSchedule, Video};

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

    #[test]
    fn initially_enabled_cameras_sample_at_offset_zero() {
        let session = session(
            true,
            Duration::from_secs(3),
            Duration::from_secs(10),
            vec![],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 3, 6, 9]));
    }

    #[test]
    fn initially_disabled_cameras_wait_until_enabled() {
        let session = session(
            false,
            Duration::from_secs(3),
            Duration::from_secs(10),
            vec![participation(4, true)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[4, 7]));
    }

    #[test]
    fn disabling_at_an_offset_removes_that_offsets_sample() {
        let session = session(
            true,
            Duration::from_secs(3),
            Duration::from_secs(10),
            vec![participation(6, false)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 3]));
    }

    #[test]
    fn enabling_samples_immediately_and_starts_a_new_cadence() {
        let session = session(
            true,
            Duration::from_secs(3),
            Duration::from_secs(10),
            vec![participation(2, false), participation(5, true)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 5, 8]));
    }

    #[test]
    fn interval_changes_sample_immediately_and_start_a_new_cadence() {
        let session = session(
            true,
            Duration::from_secs(3),
            Duration::from_secs(10),
            vec![interval(4, 2)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 3, 4, 6, 8]));
    }

    #[test]
    fn same_offset_events_are_applied_before_sampling() {
        let session = session(
            true,
            Duration::from_secs(5),
            Duration::from_secs(10),
            vec![
                participation(5, false),
                interval(5, 2),
                participation(5, true),
            ],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 5, 7, 9]));
    }

    #[test]
    fn repeated_state_and_interval_are_no_ops_for_cadence() {
        let session = session(
            true,
            Duration::from_secs(4),
            Duration::from_secs(10),
            vec![participation(3, true), interval(5, 4)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 4, 8]));
    }

    #[test]
    fn same_offset_participation_changes_that_cancel_are_a_no_op() {
        let session = session(
            true,
            Duration::from_secs(4),
            Duration::from_secs(10),
            vec![participation(3, false), participation(3, true)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 4, 8]));
    }

    #[test]
    fn same_offset_interval_changes_that_cancel_are_a_no_op() {
        let session = session(
            true,
            Duration::from_secs(4),
            Duration::from_secs(10),
            vec![interval(3, 2), interval(3, 4)],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 4, 8]));
    }

    #[test]
    fn session_end_is_exclusive() {
        let session = session(
            true,
            Duration::from_secs(5),
            Duration::from_secs(10),
            vec![],
        );

        assert_eq!(sample_offsets(&session), seconds(&[0, 5]));
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
    fn generated_offsets_are_ordered_and_unique() {
        let session = session(
            true,
            Duration::from_secs(3),
            Duration::from_secs(12),
            vec![
                interval(4, 2),
                participation(8, false),
                participation(9, true),
            ],
        );

        let offsets = sample_offsets(&session);

        assert!(offsets.windows(2).all(|offsets| offsets[0] < offsets[1]));
    }

    #[test]
    fn zero_sampling_intervals_are_rejected() {
        let session = session(true, Duration::ZERO, Duration::from_secs(10), vec![]);

        assert!(SamplingSchedule::from_session(&session, 1).is_err());
    }

    #[test]
    fn sequences_keep_global_indices_and_recording_offsets_across_videos() {
        let session = session(true, Duration::from_secs(2), Duration::from_secs(8), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let videos = vec![
            Video {
                recording_id: 10,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS - 1_000,
                end_utc_ms: SESSION_START_UTC_MS + 4_000,
            },
            Video {
                recording_id: 20,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS + 4_000,
                end_utc_ms: SESSION_START_UTC_MS + 8_000,
            },
        ];

        let sequence = SampleSequence::from_videos(SESSION_START_UTC_MS, &schedule, &videos)
            .expect("videos should cover every sample");

        assert_eq!(
            sequence,
            SampleSequence {
                camera_id: 1,
                frames: vec![
                    Frame {
                        camera_id: 1,
                        recording_id: 10,
                        sample_index: 0,
                        session_offset: Duration::ZERO,
                        recording_offset: Duration::from_secs(1),
                    },
                    Frame {
                        camera_id: 1,
                        recording_id: 10,
                        sample_index: 1,
                        session_offset: Duration::from_secs(2),
                        recording_offset: Duration::from_secs(3),
                    },
                    Frame {
                        camera_id: 1,
                        recording_id: 20,
                        sample_index: 2,
                        session_offset: Duration::from_secs(4),
                        recording_offset: Duration::ZERO,
                    },
                    Frame {
                        camera_id: 1,
                        recording_id: 20,
                        sample_index: 3,
                        session_offset: Duration::from_secs(6),
                        recording_offset: Duration::from_secs(2),
                    },
                ],
            }
        );
    }

    #[test]
    fn sequences_reject_missing_recording_coverage() {
        let session = session(true, Duration::from_secs(2), Duration::from_secs(5), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let videos = vec![
            Video {
                recording_id: 10,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS,
                end_utc_ms: SESSION_START_UTC_MS + 2_000,
            },
            Video {
                recording_id: 20,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS + 3_000,
                end_utc_ms: SESSION_START_UTC_MS + 5_000,
            },
        ];

        let error = SampleSequence::from_videos(SESSION_START_UTC_MS, &schedule, &videos)
            .expect_err("the sample at two seconds has no recording");

        assert!(error.to_string().contains("no recording"));
    }

    #[test]
    fn sequences_reject_overlapping_recording_coverage() {
        let session = session(true, Duration::from_secs(2), Duration::from_secs(5), vec![]);
        let schedule =
            SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
        let videos = vec![
            Video {
                recording_id: 10,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS,
                end_utc_ms: SESSION_START_UTC_MS + 3_000,
            },
            Video {
                recording_id: 20,
                camera_id: 1,
                start_utc_ms: SESSION_START_UTC_MS + 2_000,
                end_utc_ms: SESSION_START_UTC_MS + 5_000,
            },
        ];

        let error = SampleSequence::from_videos(SESSION_START_UTC_MS, &schedule, &videos)
            .expect_err("the sample at two seconds has two recordings");

        assert!(error.to_string().contains("multiple recordings"));
    }

    #[test]
    fn sequences_reject_utc_timestamp_overflow() {
        let schedule = SamplingSchedule {
            camera_id: 1,
            periods: vec![SamplingPeriod {
                start: Duration::from_millis(1),
                end: Duration::from_millis(2),
                sample_every: Duration::from_millis(1),
            }],
        };

        let error = SampleSequence::from_videos(i64::MAX, &schedule, &[])
            .expect_err("the session anchor and offset should be checked");

        assert!(error.to_string().contains("UTC timestamp"));
    }

    fn frame(camera_id: u32, sample_index: usize, offset_secs: u64) -> Frame {
        let offset = Duration::from_secs(offset_secs);
        Frame {
            camera_id,
            recording_id: u64::from(camera_id) * 10,
            sample_index,
            session_offset: offset,
            recording_offset: offset,
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
    fn frame_sets_sort_unsorted_input_sequences_by_camera_id() {
        let frame_sets = FrameSet::from_sequences(vec![
            sequence(3, &[0]),
            sequence(1, &[0]),
            sequence(2, &[0]),
        ])
        .expect("input sequence order should not affect output");

        assert_eq!(
            frame_sets[0]
                .frames
                .iter()
                .map(|frame| frame.camera_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
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

    #[test]
    fn frame_sets_reject_frames_out_of_session_offset_order() {
        let error = FrameSet::from_sequences(vec![sequence(1, &[2, 1])])
            .expect_err("the peekable merge requires each sequence to be ordered");

        assert!(error.to_string().contains("not ordered"));
    }
}
