use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use rig_core::{
    completion::Message,
    message::{DocumentSourceKind, UserContent},
    test_utils::{MockCompletionModel, MockTurn},
};
use uuid::Uuid;

use crate::{
    analysis::{
        agent::{Agent, AnalysisResponse, ChecklistProgress, Observation},
        analyzer::AnalysisCheckpoint,
        video::{AnalysisWarning, Frame, FrameSet},
    },
    recording::RecordingSegment,
    session::{Session, SessionCamera},
};

use super::{
    Analyzer, append_prompt_frame, append_prompt_frame_set, format_timestamp, plan_fingerprint,
    prompt_content,
};

const SESSION_START_UTC_MS: i64 = 1_786_204_800_000;

fn session(cameras: Vec<SessionCamera>) -> Session {
    Session {
        id: Uuid::from_u128(1),
        start_utc_ms: SESSION_START_UTC_MS,
        end_offset: Duration::from_secs(5),
        cameras,
        actions: Vec::new(),
    }
}

fn camera(id: u32, enabled: bool, sample_every_secs: u64) -> SessionCamera {
    SessionCamera {
        id,
        name: format!("Camera {id}"),
        enabled,
        sample_every: Duration::from_secs(sample_every_secs),
    }
}

fn segment(
    camera_id: u32,
    start_offset_ms: i64,
    end_offset_ms: i64,
    path: PathBuf,
) -> RecordingSegment {
    RecordingSegment {
        camera_id,
        start_utc_ms: SESSION_START_UTC_MS + start_offset_ms,
        end_utc_ms: SESSION_START_UTC_MS + end_offset_ms,
        path,
    }
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../camera/fixtures/default.mp4")
}

fn covering_segment(camera_id: u32) -> RecordingSegment {
    segment(camera_id, 0, 5_000, fixture_path())
}

fn fingerprint_plan(path: PathBuf) -> Vec<FrameSet> {
    vec![FrameSet {
        session_offset: Duration::from_millis(1_000),
        frames: vec![Frame {
            camera_id: 2,
            segment_start_utc_ms: 1_786_204_800_000,
            segment_end_utc_ms: 1_786_204_805_000,
            sample_index: 3,
            session_offset: Duration::from_millis(1_000),
            recording_offset: Duration::from_millis(250),
            path,
        }],
    }]
}

async fn resume_analyzer(checkpoint: PathBuf, frame_sets_per_batch: usize) -> Analyzer {
    Analyzer::resume(
        vec![covering_segment(1)],
        session(vec![camera(1, true, 2)]),
        "Start the exercise".into(),
        NonZeroUsize::new(frame_sets_per_batch).unwrap(),
        0,
        checkpoint,
    )
    .await
    .expect("analysis plan should resume")
}

fn response(summary: &str) -> AnalysisResponse {
    AnalysisResponse {
        observations: vec![Observation {
            timestamp: "00:00:01".into(),
            description: "The student starts the exercise.".into(),
        }],
        sequence_summary: summary.into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Start the exercise".into(),
            status: "respected".into(),
            note: String::new(),
        }],
    }
}

#[test]
fn prompt_preserves_previous_response_and_frame_order() {
    let previous = response("The first step has started.");
    let mut content =
        prompt_content("Open the valve", Some(&previous)).expect("prompt header should be built");

    append_prompt_frame_set(&mut content, "00:00:01.000");
    append_prompt_frame(&mut content, 1, "Front", "00:00:01.000", &[1, 2]);
    append_prompt_frame(&mut content, 2, "Side", "00:00:01.000", &[3]);
    append_prompt_frame_set(&mut content, "00:00:02.000");
    append_prompt_frame(&mut content, 1, "Front", "00:00:02.000", &[4]);

    let content = content.iter().collect::<Vec<_>>();
    assert!(matches!(
        content[0],
        UserContent::Text(text) if text.text.contains("Open the valve")
    ));
    assert!(matches!(
        content[1],
        UserContent::Text(text) if text.text.contains("The first step has started.")
    ));
    assert!(matches!(
        content[2],
        UserContent::Text(text) if text.text.contains("00:00:01.000")
    ));
    assert!(matches!(
        content[3],
        UserContent::Text(text)
            if text.text.contains("camera 1")
                && text.text.contains("Front")
                && text.text.contains("00:00:01.000")
    ));
    assert!(matches!(
        content[4],
        UserContent::Image(image)
            if image.data == DocumentSourceKind::Base64("AQI=".into())
    ));
    assert!(matches!(
        content[5],
        UserContent::Text(text)
            if text.text.contains("camera 2") && text.text.contains("Side")
    ));
    assert!(matches!(
        content[6],
        UserContent::Image(image)
            if image.data == DocumentSourceKind::Base64("Aw==".into())
    ));
    assert!(matches!(
        content[7],
        UserContent::Text(text) if text.text.contains("00:00:02.000")
    ));
    assert!(matches!(
        content[8],
        UserContent::Text(text) if text.text.contains("camera 1")
    ));
    assert!(matches!(
        content[9],
        UserContent::Image(image)
            if image.data == DocumentSourceKind::Base64("BA==".into())
    ));
}

#[test]
fn session_timestamps_include_zero_padded_milliseconds() {
    assert_eq!(
        format_timestamp(Duration::from_millis(3_723_004)),
        "01:02:03.004"
    );
}

#[tokio::test]
async fn initial_checkpoint_exists_before_provider_or_extraction_failure() {
    let extraction_directory = tempfile::tempdir().expect("temporary directory should be created");
    let extraction_checkpoint = extraction_directory.path().join("analysis.json");
    let invalid_segment = extraction_directory.path().join("invalid.mkv");
    std::fs::write(&invalid_segment, b"not valid video media")
        .expect("invalid local segment should be written");
    let extraction_model = MockCompletionModel::text(
        serde_json::to_string(&response("unused")).expect("response should serialize"),
    );
    let recorded_model = extraction_model.clone();
    let mut extraction_analyzer = Analyzer::resume(
        vec![segment(1, 0, 5_000, invalid_segment)],
        session(vec![camera(1, true, 2)]),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        extraction_checkpoint.clone(),
    )
    .await
    .expect("invalid media should not prevent local planning");
    assert!(
        AnalysisCheckpoint::read(&extraction_checkpoint, Uuid::from_u128(1))
            .expect("initial extraction checkpoint should be readable")
            .responses
            .is_empty()
    );

    let result = extraction_analyzer
        .analyze_next(&Agent::new(extraction_model))
        .await;

    assert!(matches!(result, Err(super::Error::Video(_))));
    assert!(recorded_model.requests().is_empty());
    assert_eq!(extraction_analyzer.next_batch_index(), 0);
    assert!(
        AnalysisCheckpoint::read(&extraction_checkpoint, Uuid::from_u128(1))
            .expect("extraction failure should preserve initial checkpoint")
            .responses
            .is_empty()
    );

    let provider_directory = tempfile::tempdir().expect("temporary directory should be created");
    let provider_checkpoint = provider_directory.path().join("analysis.json");
    let mut provider_analyzer = resume_analyzer(provider_checkpoint.clone(), 2).await;
    assert!(
        AnalysisCheckpoint::read(&provider_checkpoint, Uuid::from_u128(1))
            .expect("initial provider checkpoint should be readable")
            .responses
            .is_empty()
    );
    let provider = Agent::new(MockCompletionModel::new([MockTurn::error(
        "provider unavailable",
    )]));

    let result = provider_analyzer
        .submit_prompt(&provider, Message::user("prebuilt prompt"))
        .await;

    assert!(result.is_err());
    assert_eq!(provider_analyzer.next_batch_index(), 0);
    assert!(
        AnalysisCheckpoint::read(&provider_checkpoint, Uuid::from_u128(1))
            .expect("provider failure should preserve initial checkpoint")
            .responses
            .is_empty()
    );
}

#[tokio::test]
async fn failed_checkpoint_save_rolls_back_the_completed_batch() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint_directory = directory.path().join("checkpoint");
    std::fs::create_dir(&checkpoint_directory).expect("checkpoint directory should be created");
    let checkpoint = checkpoint_directory.join("analysis.json");
    let expected = response("The first batch is complete.");
    let model = MockCompletionModel::text(
        serde_json::to_string(&expected).expect("response should serialize"),
    );
    let agent = Agent::new(model);
    let mut analyzer = resume_analyzer(checkpoint.clone(), 2).await;
    std::fs::remove_file(&checkpoint).expect("initial checkpoint should be removed");
    std::fs::remove_dir(&checkpoint_directory).expect("checkpoint directory should be removed");

    let result = analyzer
        .submit_prompt(&agent, Message::user("prebuilt prompt"))
        .await;

    assert!(result.is_err());
    assert_eq!(analyzer.next_batch_index(), 0);
    assert!(!checkpoint.exists());
}

#[tokio::test]
async fn completed_analysis_rejects_another_batch() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let expected = response("The analysis is complete.");
    let model = MockCompletionModel::text(
        serde_json::to_string(&expected).expect("response should serialize"),
    );
    let agent = Agent::new(model);
    let mut analyzer = resume_analyzer(checkpoint.clone(), 10).await;

    let actual = analyzer
        .submit_prompt(&agent, Message::user("prebuilt prompt"))
        .await
        .expect("only batch should complete");
    assert_eq!(actual, &expected);
    assert_eq!(analyzer.next_batch_index(), 1);
    assert!(checkpoint.exists());

    let result = analyzer.analyze_next(&agent).await;
    assert!(matches!(
        result,
        Err(super::Error::AnalysisComplete { total: 1 })
    ));
}

#[tokio::test]
async fn resume_starts_at_the_first_incomplete_batch_with_previous_context() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let first = response("The first batch is complete.");
    let first_model = MockCompletionModel::text(
        serde_json::to_string(&first).expect("response should serialize"),
    );
    let first_agent = Agent::new(first_model);
    let mut first_analyzer = resume_analyzer(checkpoint.clone(), 2).await;
    first_analyzer
        .submit_prompt(&first_agent, Message::user("prebuilt prompt"))
        .await
        .expect("first batch should complete");
    drop(first_analyzer);

    let resumed = resume_analyzer(checkpoint, 2).await;

    assert_eq!(resumed.next_batch_index(), 1);
    let content = prompt_content(&resumed.checklist, resumed.checkpoint.responses.last())
        .expect("resumed prompt should be built");
    assert!(content.iter().any(|content| matches!(
        content,
        UserContent::Text(text) if text.text.contains("The first batch is complete.")
    )));
}

#[tokio::test]
#[ignore = "requires FFmpeg on PATH"]
async fn full_local_ffmpeg_and_model_pipeline_uses_the_existing_fixture() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let expected = response("The batch is complete.");
    let model = MockCompletionModel::text(
        serde_json::to_string(&expected).expect("response should serialize"),
    );
    let recorded_model = model.clone();
    let agent = Agent::new(model);
    let mut exercise = session(vec![camera(1, true, 1), camera(2, true, 1)]);
    exercise.end_offset = Duration::from_secs(2);
    let mut analyzer = Analyzer::resume(
        vec![
            segment(1, 0, 3_000, fixture_path()),
            segment(2, 0, 3_000, fixture_path()),
        ],
        exercise,
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await
    .expect("analysis plan should resume");

    let actual = analyzer
        .analyze_next(&agent)
        .await
        .expect("fixture batch should be analyzed");

    assert_eq!(actual, &expected);
    assert!(checkpoint.exists());
    let requests = recorded_model.requests();
    let Message::User { content } = requests[0]
        .chat_history
        .iter()
        .last()
        .expect("request should contain a user message")
    else {
        panic!("last request message should be from the user");
    };
    let content = content.iter().collect::<Vec<_>>();
    assert!(matches!(
        content[2],
        UserContent::Text(text) if text.text.contains("00:00:00.000")
    ));
    assert!(matches!(
        content[3],
        UserContent::Text(text) if text.text.contains("camera 1")
    ));
    assert!(matches!(content[4], UserContent::Image(_)));
    assert!(matches!(
        content[5],
        UserContent::Text(text) if text.text.contains("camera 2")
    ));
    assert!(matches!(content[6], UserContent::Image(_)));
    assert!(matches!(
        content[7],
        UserContent::Text(text) if text.text.contains("00:00:01.000")
    ));
}

#[tokio::test]
#[ignore = "requires FFmpeg on PATH"]
async fn full_local_ffmpeg_pipeline_resumes_with_previous_response_and_next_frames() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let first = response("The first batch is complete.");
    let second = response("Both batches are complete.");
    let model = MockCompletionModel::new([
        MockTurn::text(serde_json::to_string(&first).expect("response should serialize")),
        MockTurn::text(serde_json::to_string(&second).expect("response should serialize")),
    ]);
    let recorded_model = model.clone();
    let agent = Agent::new(model.clone());
    let segments = vec![
        segment(1, 0, 5_000, fixture_path()),
        segment(2, 0, 5_000, fixture_path()),
    ];
    let mut first_session = session(vec![camera(1, true, 1), camera(2, true, 1)]);
    first_session.end_offset = Duration::from_secs(4);
    let mut first_analyzer = Analyzer::resume(
        segments.clone(),
        first_session,
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await
    .expect("first analyzer should plan two batches");

    first_analyzer
        .analyze_next(&agent)
        .await
        .expect("first batch should be analyzed");
    assert_eq!(first_analyzer.next_batch_index(), 1);
    assert!(checkpoint.exists());
    drop(first_analyzer);

    let mut resumed_session = session(vec![camera(1, true, 1), camera(2, true, 1)]);
    resumed_session.end_offset = Duration::from_secs(4);
    let mut resumed = Analyzer::resume(
        segments,
        resumed_session,
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint,
    )
    .await
    .expect("second analyzer should resume the saved plan");

    assert_eq!(resumed.next_batch_index(), 1);
    resumed
        .analyze_next(&Agent::new(model))
        .await
        .expect("second batch should be analyzed");
    assert_eq!(resumed.next_batch_index(), 2);

    let requests = recorded_model.requests();
    assert_eq!(requests.len(), 2);
    let Message::User { content } = requests[1]
        .chat_history
        .iter()
        .last()
        .expect("request should contain a user message")
    else {
        panic!("last request message should be from the user");
    };
    let content = content.iter().collect::<Vec<_>>();
    assert!(matches!(
        content[1],
        UserContent::Text(text)
            if text.text == format!(
                "Previous complete analysis response:\n{}",
                serde_json::to_string(&first).unwrap()
            )
    ));
    assert!(matches!(
        content[2],
        UserContent::Text(text) if text.text.contains("00:00:02.000")
    ));
    assert!(matches!(
        content[3],
        UserContent::Text(text) if text.text.contains("camera 1")
    ));
    assert!(matches!(content[4], UserContent::Image(_)));
    assert!(matches!(
        content[5],
        UserContent::Text(text) if text.text.contains("camera 2")
    ));
    assert!(matches!(content[6], UserContent::Image(_)));
    assert!(matches!(
        content[7],
        UserContent::Text(text) if text.text.contains("00:00:03.000")
    ));
}

#[tokio::test]
async fn resume_rejects_changed_checklist_plan_batching_or_warnings() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let exercise = || session(vec![camera(1, true, 2), camera(2, false, 1)]);
    Analyzer::resume(
        vec![covering_segment(1)],
        exercise(),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await
    .expect("initial plan should be checkpointed");

    let changed_checklist = Analyzer::resume(
        vec![covering_segment(1)],
        exercise(),
        "Use a different checklist".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await;
    assert!(matches!(
        changed_checklist,
        Err(super::Error::CheckpointChecklist)
    ));

    let changed_plan = Analyzer::resume(
        vec![segment(1, -100, 5_000, fixture_path())],
        exercise(),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await;
    assert!(matches!(
        changed_plan,
        Err(super::Error::CheckpointPlanFingerprint)
    ));

    let changed_batch_size = Analyzer::resume(
        vec![covering_segment(1)],
        exercise(),
        "Start the exercise".into(),
        NonZeroUsize::new(3).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await;
    assert!(matches!(
        changed_batch_size,
        Err(super::Error::CheckpointPlanFingerprint)
    ));

    let changed_overlap = Analyzer::resume(
        vec![covering_segment(1)],
        exercise(),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        1,
        checkpoint.clone(),
    )
    .await;
    assert!(matches!(
        changed_overlap,
        Err(super::Error::CheckpointPlanFingerprint)
    ));

    let changed_warnings = Analyzer::resume(
        vec![covering_segment(1), covering_segment(2)],
        exercise(),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint,
    )
    .await;
    assert!(matches!(
        changed_warnings,
        Err(super::Error::CheckpointWarnings)
    ));
}

#[test]
fn fingerprint_is_independent_of_absolute_paths() {
    let first = plan_fingerprint(
        &fingerprint_plan(PathBuf::from("/first/location/segment.mkv")),
        NonZeroUsize::new(5).unwrap(),
        0,
    )
    .expect("first plan should be fingerprinted");
    let second = plan_fingerprint(
        &fingerprint_plan(PathBuf::from("/different/location/segment.mkv")),
        NonZeroUsize::new(5).unwrap(),
        0,
    )
    .expect("second plan should be fingerprinted");

    assert_eq!(first, second);
}

#[test]
fn fingerprint_encoding_is_stable() {
    let fingerprint = plan_fingerprint(
        &fingerprint_plan(PathBuf::from("/excluded/from/fingerprint.mkv")),
        NonZeroUsize::new(5).unwrap(),
        0,
    )
    .expect("golden plan should be fingerprinted");

    assert_eq!(
        fingerprint,
        "a3d35b83f408534773b83f3acb173d660dfd9ec20af1c5dd94e0dea4e0528a29"
    );
}

#[test]
fn fingerprint_changes_when_only_overlap_changes() {
    let frame_sets = fingerprint_plan(PathBuf::from("/excluded/from/fingerprint.mkv"));
    let without_overlap = plan_fingerprint(&frame_sets, NonZeroUsize::new(5).unwrap(), 0).unwrap();
    let with_overlap = plan_fingerprint(&frame_sets, NonZeroUsize::new(5).unwrap(), 2).unwrap();

    assert_ne!(without_overlap, with_overlap);
}

#[tokio::test]
async fn resume_plans_overlapping_batches_with_one_final_partial_batch() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let mut exercise = session(vec![camera(1, true, 1)]);
    exercise.end_offset = Duration::from_secs(10);
    let analyzer = Analyzer::resume(
        vec![segment(1, 0, 10_000, fixture_path())],
        exercise,
        "Start the exercise".into(),
        NonZeroUsize::new(5).unwrap(),
        2,
        directory.path().join("analysis.json"),
    )
    .await
    .expect("overlapping analysis plan should start");

    assert_eq!(analyzer.frame_sets.len(), 10);
    assert_eq!(analyzer.checkpoint.total_batches, 3);
    assert_eq!(
        (0..analyzer.checkpoint.total_batches)
            .map(|index| analyzer.batch_range(index))
            .collect::<Vec<_>>(),
        vec![0..5, 3..8, 6..10]
    );
}

#[tokio::test]
async fn resume_rebuilds_the_canonical_plan_and_zero_overlap_batches() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let checkpoint = directory.path().join("analysis.json");
    let analyzer = Analyzer::resume(
        vec![covering_segment(1)],
        session(vec![camera(1, true, 2), camera(2, false, 1)]),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        checkpoint.clone(),
    )
    .await
    .expect("new analysis should start");

    assert_eq!(
        analyzer
            .frame_sets
            .iter()
            .map(|frame_set| frame_set.session_offset)
            .collect::<Vec<_>>(),
        [0, 2, 4].map(Duration::from_secs)
    );
    assert_eq!(
        analyzer.checkpoint.warnings,
        vec![AnalysisWarning::RecordingGap {
            camera_id: 2,
            start_offset_ms: 0,
            end_offset_ms: 5_000,
        }]
    );
    assert_eq!(analyzer.frame_sets_per_batch.get(), 2);
    assert_eq!(analyzer.checkpoint.total_batches, 2);
    assert_eq!(analyzer.batch_range(0), 0..2);
    assert_eq!(analyzer.batch_range(1), 2..3);
    assert_eq!(analyzer.next_batch_index(), 0);
    assert_eq!(
        AnalysisCheckpoint::read(&checkpoint, Uuid::from_u128(1))
            .expect("initial checkpoint should be readable"),
        analyzer.checkpoint
    );
}

#[tokio::test]
async fn resume_rejects_overlap_equal_to_batch_size() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");

    let result = Analyzer::resume(
        vec![covering_segment(1)],
        session(vec![camera(1, true, 1)]),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        2,
        directory.path().join("analysis.json"),
    )
    .await;

    assert!(matches!(result, Err(super::Error::InvalidBatchOverlap)));
}

#[tokio::test]
async fn resume_rejects_an_empty_merged_plan() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");

    let result = Analyzer::resume(
        Vec::new(),
        session(vec![camera(1, true, 1)]),
        "Start the exercise".into(),
        NonZeroUsize::new(2).unwrap(),
        0,
        directory.path().join("analysis.json"),
    )
    .await;

    assert!(matches!(result, Err(super::Error::NoAnalyzableFrames)));
}
