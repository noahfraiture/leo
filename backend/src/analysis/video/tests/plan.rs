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

    let schedule = SamplingSchedule::from_session(&session, 1).expect("schedule should be built");

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
    let schedule = SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
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
    let schedule = SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
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
    let schedule = SamplingSchedule::from_session(&session, 1).expect("schedule should be built");
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
        .map(|schedule| SampleSequence::from_segments(SESSION_START_UTC_MS, schedule, &segments))
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
