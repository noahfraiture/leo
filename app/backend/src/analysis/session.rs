//! Validates one completed session and drives its resumable analysis to completion.

use std::{fs, num::NonZeroUsize, path::PathBuf};

use rig_core::completion::CompletionModel;

use crate::{recording::list_segments, session::Session};

use super::{
    agent::{Agent, OpenAiConfiguration},
    analyzer::{AnalysisCheckpoint, AnalysisIdentity, Analyzer},
    error::Error,
};

type Result<T> = std::result::Result<T, Error>;

/// A request to analyze one completed session directory against a checklist.
pub struct AnalyzeSession {
    /// Directory containing the event log, completion marker, and local recordings.
    pub directory: PathBuf,
    /// Correct exercise sequence supplied to every model request.
    pub checklist: String,
}

/// Analyzes or resumes a completed local session and emits each durable checkpoint snapshot.
pub async fn analyze_session(
    request: AnalyzeSession,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint> {
    let configuration = OpenAiConfiguration::from_env()?;
    let analysis_identity = configuration.identity().clone();
    analyze_session_with(
        request,
        analysis_identity,
        || configuration.into_agent().map_err(Error::from),
        on_checkpoint,
    )
    .await
}

async fn analyze_session_with<M, F>(
    request: AnalyzeSession,
    analysis_identity: AnalysisIdentity,
    make_agent: F,
    mut on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint>
where
    M: CompletionModel,
    F: FnOnce() -> Result<Agent<M>>,
{
    let AnalyzeSession {
        directory,
        checklist,
    } = request;
    if checklist.trim().is_empty() {
        return Err(Error::EmptyChecklist);
    }

    if !fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return Err(Error::InvalidSessionDirectory);
    }
    if !fs::symlink_metadata(directory.join("recording-complete"))
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
    {
        return Err(Error::InvalidCompletionMarker);
    }

    let session = Session::load(&directory.join("events.jsonl"))?;
    let camera_ids = session
        .cameras
        .iter()
        .map(|camera| camera.id)
        .collect::<Vec<_>>();
    let recordings_root = directory.join("recordings");
    let segments =
        tokio::task::spawn_blocking(move || list_segments(&recordings_root, &camera_ids))
            .await
            .map_err(Error::SegmentDiscoveryTask)??;
    let checkpoint_path = directory.join("analysis.json");
    let checklist = match fs::symlink_metadata(&checkpoint_path) {
        Ok(_) => checklist,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => checklist.trim().to_owned(),
        Err(error) => return Err(error.into()),
    };
    let mut analyzer = Analyzer::resume(
        segments,
        session,
        checklist,
        analysis_identity,
        NonZeroUsize::new(5).expect("analysis batch size is non-zero"),
        checkpoint_path,
    )
    .await?;

    let mut checkpoint = analyzer.checkpoint().clone();
    on_checkpoint(checkpoint.clone());
    if checkpoint.responses.len() == checkpoint.total_batches {
        return Ok(checkpoint);
    }

    let agent = make_agent()?;
    while checkpoint.responses.len() < checkpoint.total_batches {
        analyzer.analyze_next(&agent).await?;
        checkpoint = analyzer.checkpoint().clone();
        on_checkpoint(checkpoint.clone());
    }
    Ok(checkpoint)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, path::Path};

    use rig_core::{
        completion::Message,
        message::{DocumentSourceKind, ImageMediaType, UserContent},
        test_utils::{MockCompletionModel, MockTurn},
    };
    use serde_json::json;
    use uuid::Uuid;

    use crate::analysis::{
        AnalysisWarning, Error,
        agent::{Agent, AnalysisResponse, ChecklistProgress, Observation},
        analyzer::{AnalysisCheckpoint, AnalysisIdentity},
    };

    use super::{AnalyzeSession, analyze_session_with};

    const SESSION_ID: &str = "5a660250-36fc-4c2b-93fa-b04247bdad20";
    const START_UTC_MS: i64 = 1_786_204_800_000;

    async fn analyze_with_mock(
        request: AnalyzeSession,
        analysis_identity: AnalysisIdentity,
        model: MockCompletionModel,
        constructions: &Cell<usize>,
        on_checkpoint: impl FnMut(AnalysisCheckpoint),
    ) -> Result<AnalysisCheckpoint, Error> {
        analyze_session_with(
            request,
            analysis_identity,
            || {
                constructions.set(constructions.get() + 1);
                Ok(Agent::new(model))
            },
            on_checkpoint,
        )
        .await
    }

    fn analysis_identity() -> AnalysisIdentity {
        AnalysisIdentity {
            model: "safe-model-byte-sentinel".into(),
            endpoint_id: "safe-endpoint-byte-sentinel".into(),
        }
    }

    fn response(summary: &str) -> AnalysisResponse {
        AnalysisResponse {
            observations: vec![Observation {
                timestamp: "00:00:00.000".into(),
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

    fn model(responses: impl IntoIterator<Item = AnalysisResponse>) -> MockCompletionModel {
        MockCompletionModel::new(responses.into_iter().map(|response| {
            MockTurn::text(serde_json::to_string(&response).expect("response should serialize"))
        }))
    }

    fn write_session(directory: &Path, end_offset_ms: u64) {
        fs::create_dir_all(directory.join("recordings/camera-1"))
            .expect("recording directory should be created");
        let events = [
            json!({
                "schema_version": 1,
                "sequence": 0,
                "session_id": SESSION_ID,
                "utc_ms": START_UTC_MS,
                "session_offset_ms": 0,
                "action": {
                    "type": "session_started",
                    "cameras": [{
                        "camera_id": 1,
                        "name": "Camera 1",
                        "enabled": true,
                        "sample_every_ms": 1_000
                    }]
                }
            }),
            json!({
                "schema_version": 1,
                "sequence": 1,
                "session_id": SESSION_ID,
                "utc_ms": START_UTC_MS + i64::try_from(end_offset_ms).unwrap(),
                "session_offset_ms": end_offset_ms,
                "action": { "type": "session_ended" }
            }),
        ];
        let contents = events
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .expect("events should serialize")
            .join("\n")
            + "\n";
        fs::write(directory.join("events.jsonl"), contents).expect("event log should be written");
    }

    fn mark_complete(directory: &Path) {
        fs::write(directory.join("recording-complete"), b"")
            .expect("completion marker should be written");
    }

    fn add_segment(directory: &Path, start_offset_ms: i64) {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../camera/fixtures/default.mp4");
        fs::copy(
            fixture,
            directory
                .join("recordings/camera-1")
                .join(format!("{}.mkv", START_UTC_MS + start_offset_ms)),
        )
        .expect("recording segment should be copied");
    }

    fn assert_no_session_local_temporary_media(directory: &Path) {
        for entry in fs::read_dir(directory).expect("session directory should be readable") {
            let entry = entry.expect("session entry should be readable");
            let path = entry.path();
            if entry
                .file_type()
                .expect("session entry type should be readable")
                .is_dir()
            {
                assert_no_session_local_temporary_media(&path);
                continue;
            }
            let extension = path.extension().and_then(|extension| extension.to_str());
            assert!(
                !extension.is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("jpg")
                        || extension.eq_ignore_ascii_case("jpeg")
                        || extension.eq_ignore_ascii_case("mp4")
                }),
                "analysis left temporary media in the session: {}",
                path.display()
            );
        }
    }

    fn request(directory: &Path, checklist: &str) -> AnalyzeSession {
        AnalyzeSession {
            directory: directory.to_owned(),
            checklist: checklist.into(),
        }
    }

    async fn create_completed_checkpoint(directory: &Path) -> AnalysisCheckpoint {
        write_session(directory, 1_000);
        mark_complete(directory);
        add_segment(directory, 0);
        let constructions = Cell::new(0);
        let checkpoint = analyze_with_mock(
            request(directory, "Start the exercise"),
            analysis_identity(),
            model([response("Analysis complete.")]),
            &constructions,
            |_| {},
        )
        .await
        .expect("initial analysis should complete");
        assert_eq!(constructions.get(), 1);
        checkpoint
    }

    fn persist_checkpoint(directory: &Path, checkpoint: &AnalysisCheckpoint) -> Vec<u8> {
        let mut contents =
            serde_json::to_vec_pretty(checkpoint).expect("checkpoint should serialize");
        contents.push(b'\n');
        fs::write(directory.join("analysis.json"), &contents)
            .expect("checkpoint should be written");
        contents
    }

    #[tokio::test]
    async fn empty_checklist_fails_before_filesystem_access() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let result = analyze_with_mock(
            request(&directory.path().join("missing"), " \n\t "),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;

        assert!(matches!(result, Err(Error::EmptyChecklist)));
        assert_eq!(constructions.get(), 0);
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn missing_or_symlinked_marker_is_rejected() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let missing = root.path().join("missing-marker");
        write_session(&missing, 1_000);
        let constructions = Cell::new(0);
        let missing_result = analyze_with_mock(
            request(&missing, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |_| {},
        )
        .await;
        assert!(matches!(
            missing_result,
            Err(Error::InvalidCompletionMarker)
        ));

        let nonempty = root.path().join("nonempty-marker");
        write_session(&nonempty, 1_000);
        fs::write(nonempty.join("recording-complete"), b"not empty")
            .expect("nonempty marker should be written");
        let nonempty_result = analyze_with_mock(
            request(&nonempty, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |_| {},
        )
        .await;
        assert!(matches!(
            nonempty_result,
            Err(Error::InvalidCompletionMarker)
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let symlinked = root.path().join("symlinked-marker");
            write_session(&symlinked, 1_000);
            let marker_target = root.path().join("marker-target");
            fs::write(&marker_target, b"").expect("marker target should be written");
            symlink(&marker_target, symlinked.join("recording-complete"))
                .expect("marker symlink should be created");
            let symlinked_result = analyze_with_mock(
                request(&symlinked, "Start the exercise"),
                analysis_identity(),
                MockCompletionModel::default(),
                &constructions,
                |_| {},
            )
            .await;
            assert!(matches!(
                symlinked_result,
                Err(Error::InvalidCompletionMarker)
            ));

            let directory_target = root.path().join("directory-target");
            write_session(&directory_target, 1_000);
            mark_complete(&directory_target);
            let directory_link = root.path().join("directory-link");
            symlink(&directory_target, &directory_link)
                .expect("session directory symlink should be created");
            let directory_result = analyze_with_mock(
                request(&directory_link, "Start the exercise"),
                analysis_identity(),
                MockCompletionModel::default(),
                &constructions,
                |_| {},
            )
            .await;
            assert!(matches!(
                directory_result,
                Err(Error::InvalidSessionDirectory)
            ));
        }

        assert_eq!(constructions.get(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_events_or_checkpoint_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory should be created");
        let symlinked_events = root.path().join("symlinked-events");
        fs::create_dir(&symlinked_events).expect("session directory should be created");
        mark_complete(&symlinked_events);
        let events_target = root.path().join("events-target.jsonl");
        fs::write(&events_target, b"not read through the link\n")
            .expect("events target should be written");
        symlink(&events_target, symlinked_events.join("events.jsonl"))
            .expect("events symlink should be created");
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let events_result = analyze_with_mock(
            request(&symlinked_events, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;
        assert!(matches!(
            events_result,
            Err(Error::Session(crate::session::Error::InvalidEventFile))
        ));

        let symlinked_checkpoint = root.path().join("symlinked-checkpoint");
        write_session(&symlinked_checkpoint, 1_000);
        mark_complete(&symlinked_checkpoint);
        add_segment(&symlinked_checkpoint, 0);
        let checkpoint_target = root.path().join("checkpoint-target.json");
        fs::write(&checkpoint_target, b"{}\n").expect("checkpoint target should be written");
        symlink(
            &checkpoint_target,
            symlinked_checkpoint.join("analysis.json"),
        )
        .expect("checkpoint symlink should be created");

        let checkpoint_result = analyze_with_mock(
            request(&symlinked_checkpoint, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;
        assert!(matches!(
            checkpoint_result,
            Err(Error::InvalidCheckpointFile)
        ));
        assert_eq!(constructions.get(), 0);
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn no_analyzable_frames_fails_before_agent_construction() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 1_000);
        mark_complete(&directory);
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let result = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;

        assert!(matches!(result, Err(Error::NoAnalyzableFrames)));
        assert_eq!(constructions.get(), 0);
        assert!(snapshots.is_empty());
        assert!(!directory.join("analysis.json").exists());
    }

    #[tokio::test]
    async fn initial_identity_is_saved_before_first_provider_request() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 6_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        add_segment(&directory, 5_000);
        let expected = [
            response("First batch complete."),
            response("Analysis complete."),
        ];
        let identity = analysis_identity();
        let model = model(expected.clone());
        let recorded_model = model.clone();
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();
        let mut request_counts = Vec::new();
        let mut initial_disk_identity = None;
        let checkpoint_path = directory.join("analysis.json");

        let checkpoint = analyze_with_mock(
            request(&directory, "  Start the exercise  "),
            identity.clone(),
            model,
            &constructions,
            |checkpoint| {
                let request_count = recorded_model.request_count();
                request_counts.push(request_count);
                if request_count == 0 {
                    initial_disk_identity = Some(
                        AnalysisCheckpoint::read(
                            &checkpoint_path,
                            Uuid::parse_str(SESSION_ID).unwrap(),
                        )
                        .expect("initial checkpoint should already be durable")
                        .analysis_identity,
                    );
                }
                snapshots.push(checkpoint);
            },
        )
        .await
        .expect("analysis should complete");

        assert_eq!(constructions.get(), 1);
        assert_eq!(
            snapshots
                .iter()
                .map(|checkpoint| checkpoint.responses.len())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(request_counts, vec![0, 1, 2]);
        assert_eq!(initial_disk_identity, Some(identity.clone()));
        assert_eq!(snapshots[0].analysis_identity, identity);
        assert_eq!(checkpoint.total_batches, 2);
        assert_eq!(checkpoint.checklist, "Start the exercise");
        assert_eq!(checkpoint.responses, expected);
        assert_eq!(snapshots.last(), Some(&checkpoint));
        assert_eq!(
            AnalysisCheckpoint::read(
                &directory.join("analysis.json"),
                Uuid::parse_str(SESSION_ID).unwrap(),
            )
            .expect("saved checkpoint should be readable"),
            checkpoint
        );
    }

    #[tokio::test]
    async fn matching_identity_resumes_at_first_incomplete_batch() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 6_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        add_segment(&directory, 5_000);
        let identity = analysis_identity();
        let first = response("First batch complete.");
        let interrupted_model = MockCompletionModel::new([
            MockTurn::text(serde_json::to_string(&first).expect("first response should serialize")),
            MockTurn::error("stop after the first durable response"),
        ]);
        let initial_constructions = Cell::new(0);

        let initial = analyze_with_mock(
            request(&directory, "Start the exercise"),
            identity.clone(),
            interrupted_model,
            &initial_constructions,
            |_| {},
        )
        .await;
        assert!(initial.is_err());
        assert_eq!(initial_constructions.get(), 1);
        let partial = AnalysisCheckpoint::read(
            &directory.join("analysis.json"),
            Uuid::parse_str(SESSION_ID).unwrap(),
        )
        .expect("partial checkpoint should be readable");
        assert_eq!(partial.responses, vec![first.clone()]);

        let second = response("Analysis complete.");
        let resumed_model = model([second.clone()]);
        let recorded_model = resumed_model.clone();
        let resumed_constructions = Cell::new(0);
        let mut snapshots = Vec::new();
        let resumed = analyze_with_mock(
            request(&directory, "Start the exercise"),
            identity.clone(),
            resumed_model,
            &resumed_constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await
        .expect("matching identity should resume the incomplete batch");

        assert_eq!(resumed_constructions.get(), 1);
        assert_eq!(recorded_model.request_count(), 1);
        assert_eq!(
            snapshots
                .iter()
                .map(|checkpoint| checkpoint.responses.len())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(resumed.analysis_identity, identity);
        assert_eq!(resumed.responses, vec![first, second]);
    }

    #[tokio::test]
    async fn genuine_v2_checkpoint_schema_mismatch_suppresses_side_effects() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 1_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        let fixture = json!({
            "schema_version": 2,
            "session_id": SESSION_ID,
            "checklist": "Start the exercise",
            "plan_fingerprint": "v2-plan-fingerprint",
            "total_batches": 1,
            "warnings": [],
            "responses": []
        });
        assert!(fixture.get("analysis_identity").is_none());
        let mut saved_bytes =
            serde_json::to_vec_pretty(&fixture).expect("v2 fixture should serialize");
        saved_bytes.push(b'\n');
        fs::write(directory.join("analysis.json"), &saved_bytes)
            .expect("v2 fixture should be written");
        let model = MockCompletionModel::default();
        let recorded_model = model.clone();
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let result = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            model,
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::CheckpointSchema {
                expected: 3,
                actual: 2
            })
        ));
        assert_eq!(constructions.get(), 0);
        assert_eq!(recorded_model.request_count(), 0);
        assert!(snapshots.is_empty());
        assert_eq!(
            fs::read(directory.join("analysis.json")).expect("checkpoint should remain readable"),
            saved_bytes
        );
    }

    #[tokio::test]
    async fn identity_mismatch_preserves_bytes_and_suppresses_provider() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 6_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        add_segment(&directory, 5_000);
        let identity = analysis_identity();
        let setup_constructions = Cell::new(0);
        let mut partial = analyze_with_mock(
            request(&directory, "Start the exercise"),
            identity.clone(),
            model([
                response("First batch complete."),
                response("Analysis complete."),
            ]),
            &setup_constructions,
            |_| {},
        )
        .await
        .expect("setup analysis should complete");
        partial.responses.truncate(1);
        let saved_bytes = persist_checkpoint(&directory, &partial);

        let mismatches = [
            (
                "model",
                AnalysisIdentity {
                    model: format!(" {}", identity.model),
                    endpoint_id: identity.endpoint_id.clone(),
                },
            ),
            (
                "endpoint",
                AnalysisIdentity {
                    model: identity.model.clone(),
                    endpoint_id: format!("{} ", identity.endpoint_id),
                },
            ),
        ];
        for (field, mismatch) in mismatches {
            let model = MockCompletionModel::default();
            let recorded_model = model.clone();
            let constructions = Cell::new(0);
            let mut snapshots = Vec::new();

            let result = analyze_with_mock(
                request(&directory, "Start the exercise"),
                mismatch,
                model,
                &constructions,
                |checkpoint| snapshots.push(checkpoint),
            )
            .await;

            match field {
                "model" => assert!(matches!(result, Err(Error::CheckpointModel))),
                "endpoint" => assert!(matches!(result, Err(Error::CheckpointEndpointId))),
                _ => unreachable!(),
            }
            assert_eq!(constructions.get(), 0);
            assert_eq!(recorded_model.request_count(), 0);
            assert!(snapshots.is_empty());
            assert_eq!(
                fs::read(directory.join("analysis.json"))
                    .expect("checkpoint should remain readable"),
                saved_bytes
            );
        }
    }

    #[tokio::test]
    async fn completed_matching_checkpoint_returns_without_agent_construction() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 1_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        let first_constructions = Cell::new(0);
        let completed = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            model([response("Analysis complete.")]),
            &first_constructions,
            |_| {},
        )
        .await
        .expect("initial analysis should complete");
        assert_eq!(first_constructions.get(), 1);

        let resumed_constructions = Cell::new(0);
        let mut snapshots = Vec::new();
        let resumed = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &resumed_constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await
        .expect("completed analysis should return its checkpoint");

        assert_eq!(resumed_constructions.get(), 0);
        assert_eq!(resumed, completed);
        assert_eq!(resumed.analysis_identity, analysis_identity());
        assert_eq!(snapshots, vec![completed]);
    }

    #[tokio::test]
    async fn completed_checkpoint_resumes_exact_trailing_whitespace_without_agent_construction() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        let mut persisted = create_completed_checkpoint(&directory).await;
        persisted.checklist = "Start the exercise  \n".into();
        let saved_bytes = persist_checkpoint(&directory, &persisted);
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let resumed = analyze_with_mock(
            request(&directory, &persisted.checklist),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await
        .expect("the exact persisted checklist should resume");

        assert_eq!(constructions.get(), 0);
        assert_eq!(resumed, persisted);
        assert_eq!(snapshots, vec![persisted]);
        assert_eq!(
            fs::read(directory.join("analysis.json")).expect("checkpoint should remain readable"),
            saved_bytes
        );
    }

    #[tokio::test]
    async fn existing_checkpoint_rejects_changed_checklist_bytes() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        let mut persisted = create_completed_checkpoint(&directory).await;
        persisted.checklist = "Start the exercise  \n".into();
        let saved_bytes = persist_checkpoint(&directory, &persisted);
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let result = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            MockCompletionModel::default(),
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await;

        assert!(matches!(result, Err(Error::CheckpointChecklist)));
        assert_eq!(constructions.get(), 0);
        assert!(snapshots.is_empty());
        assert_eq!(
            fs::read(directory.join("analysis.json")).expect("checkpoint should remain readable"),
            saved_bytes
        );
    }

    #[tokio::test]
    async fn failed_save_never_emits_an_unsaved_response() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 1_000);
        mark_complete(&directory);
        add_segment(&directory, 0);
        let model = model([response("This response must not be emitted.")]);
        let recorded_model = model.clone();
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();
        let checkpoint_path = directory.join("analysis.json");

        let result = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            model,
            &constructions,
            |checkpoint| {
                snapshots.push(checkpoint);
                if snapshots.len() == 1 {
                    fs::remove_file(&checkpoint_path)
                        .expect("initial checkpoint should be removed");
                    fs::create_dir(&checkpoint_path)
                        .expect("checkpoint path should become an invalid destination");
                }
            },
        )
        .await;

        assert!(matches!(result, Err(Error::Io(_))));
        assert_eq!(constructions.get(), 1);
        assert_eq!(recorded_model.request_count(), 1);
        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].responses.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg and FFprobe from the Nix development shell"]
    async fn full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        write_session(&directory, 5_000);
        mark_complete(&directory);
        add_segment(&directory, -4_000);
        add_segment(&directory, 3_000);
        let expected = response("Pre- and post-gap frames were analyzed.");
        let model = model([expected.clone()]);
        let recorded_model = model.clone();
        let constructions = Cell::new(0);
        let mut snapshots = Vec::new();

        let checkpoint = analyze_with_mock(
            request(&directory, "Start the exercise"),
            analysis_identity(),
            model,
            &constructions,
            |checkpoint| snapshots.push(checkpoint),
        )
        .await
        .expect("local analysis should complete");

        assert_eq!(constructions.get(), 1);
        assert_eq!(
            snapshots
                .iter()
                .map(|checkpoint| checkpoint.responses.len())
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(checkpoint.total_batches, 1);
        assert_eq!(checkpoint.responses, vec![expected]);
        assert_eq!(
            checkpoint.warnings,
            vec![AnalysisWarning::RecordingGap {
                camera_id: 1,
                start_offset_ms: 1_000,
                end_offset_ms: 3_000,
            }]
        );
        assert_eq!(snapshots.last(), Some(&checkpoint));
        assert_eq!(
            AnalysisCheckpoint::read(
                &directory.join("analysis.json"),
                Uuid::parse_str(SESSION_ID).unwrap(),
            )
            .expect("saved checkpoint should be readable"),
            checkpoint
        );

        let requests = recorded_model.requests();
        assert_eq!(requests.len(), 1);
        let Message::User { content } = requests[0]
            .chat_history
            .iter()
            .last()
            .expect("request should contain a user message")
        else {
            panic!("last request message should be from the user");
        };
        let images = content
            .iter()
            .filter_map(|content| match content {
                UserContent::Image(image) => Some(image),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(images.len(), 3);
        for image in images {
            assert_eq!(image.media_type, Some(ImageMediaType::JPEG));
            assert!(matches!(
                &image.data,
                DocumentSourceKind::Base64(data) if !data.is_empty()
            ));
        }
        assert_no_session_local_temporary_media(&directory);
    }
}
