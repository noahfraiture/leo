use std::{
    io::Write,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    analysis::{
        agent::{Agent, AnalysisResponse},
        error::Error,
        video::{
            AnalysisWarning, FrameSet, SampleSequence, SamplingSchedule, extract_jpeg,
            recording_gap_warnings,
        },
    },
    recording::RecordingSegment,
    session::Session,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig_core::{
    OneOrMany,
    completion::{CompletionModel, Message},
    message::{ImageMediaType, UserContent},
};
use sha2::{Digest, Sha256};

use super::progress::{ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint};

type Result<T> = std::result::Result<T, Error>;

/// Plans local recording samples, materializes one batch at a time, and checkpoints each result.
pub struct Analyzer {
    session: Session,
    checklist: String,
    frame_sets: Vec<FrameSet>,
    frame_sets_per_batch: NonZeroUsize,
    progress_path: PathBuf,
    checkpoint: AnalysisCheckpoint,
}

impl Analyzer {
    /// Rebuilds the canonical plan and resumes from a validated checkpoint, or starts at batch zero.
    pub async fn resume(
        segments: Vec<RecordingSegment>,
        session: Session,
        checklist: String,
        frame_sets_per_batch: NonZeroUsize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        let session_id = session.id;
        tracing::info!(session_id = %session_id, "planning analysis");

        Self::resume_inner(
            segments,
            session,
            checklist,
            frame_sets_per_batch,
            progress_path,
        )
        .inspect_err(|_| tracing::error!(session_id = %session_id, "analysis planning failed"))
    }

    fn resume_inner(
        segments: Vec<RecordingSegment>,
        session: Session,
        checklist: String,
        frame_sets_per_batch: NonZeroUsize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        let mut schedules = Vec::new();
        for camera in &session.cameras {
            let schedule = SamplingSchedule::from_session(&session, camera.id)?;
            if !schedule.periods.is_empty() {
                schedules.push(schedule);
            }
        }
        let warnings = recording_gap_warnings(&session, &segments)?;
        let sequences = schedules
            .iter()
            .map(|schedule| {
                SampleSequence::from_segments(session.start_utc_ms, schedule, &segments)
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let frame_sets = FrameSet::from_sequences(sequences)?;
        if frame_sets.is_empty() {
            return Err(Error::NoAnalyzableFrames);
        }
        let plan_fingerprint = plan_fingerprint(&frame_sets, frame_sets_per_batch)?;
        let total_batches = frame_sets.chunks(frame_sets_per_batch.get()).count();
        tracing::info!(session_id = %session.id, total_batches, "analysis plan ready");
        let (checkpoint, is_new) = load_or_new(
            &progress_path,
            session.id,
            &checklist,
            &plan_fingerprint,
            total_batches,
            &warnings,
        )?;

        if is_new {
            save_checkpoint(&checkpoint, &progress_path)?;
            tracing::info!(
                session_id = %session.id,
                completed_batches = 0,
                total_batches,
                "analysis checkpoint saved"
            );
        } else {
            tracing::info!(
                session_id = %session.id,
                completed_batches = checkpoint.responses.len(),
                total_batches,
                "analysis resumed"
            );
        }
        if checkpoint.responses.len() == total_batches {
            tracing::info!(session_id = %session.id, total_batches, "analysis complete");
        }

        Ok(Self {
            session,
            checklist,
            frame_sets,
            frame_sets_per_batch,
            progress_path,
            checkpoint,
        })
    }

    /// Index the caller should materialize next after rebuilding the batch plan.
    fn next_batch_index(&self) -> usize {
        self.checkpoint.responses.len()
    }

    /// Materializes and analyzes the first incomplete batch, then durably checkpoints it.
    pub async fn analyze_next<M: CompletionModel>(
        &mut self,
        agent: &Agent<M>,
    ) -> Result<&AnalysisResponse> {
        let index = self.next_batch_index();
        if index >= self.checkpoint.total_batches {
            return Err(Error::AnalysisComplete {
                total: self.checkpoint.total_batches,
            });
        }
        tracing::info!(
            session_id = %self.session.id,
            batch_index = index,
            total_batches = self.checkpoint.total_batches,
            "analysis batch started"
        );

        let batch_size = self.frame_sets_per_batch.get();
        let start = index * batch_size;
        let end = (start + batch_size).min(self.frame_sets.len());
        let prompt = match self.materialize_prompt(&self.frame_sets[start..end]).await {
            Ok(prompt) => prompt,
            Err(error) => {
                tracing::error!(
                    session_id = %self.session.id,
                    batch_index = index,
                    total_batches = self.checkpoint.total_batches,
                    "analysis batch failed"
                );
                return Err(error);
            }
        };
        let session_id = self.session.id;
        let total_batches = self.checkpoint.total_batches;
        match self.submit_prompt(agent, prompt).await {
            Ok(response) => Ok(response),
            Err(error) => {
                tracing::error!(
                    session_id = %session_id,
                    batch_index = index,
                    total_batches,
                    "analysis batch failed"
                );
                Err(error)
            }
        }
    }

    /// Returns the complete checkpoint state most recently saved to disk.
    pub fn checkpoint(&self) -> &AnalysisCheckpoint {
        &self.checkpoint
    }

    async fn submit_prompt<M: CompletionModel>(
        &mut self,
        agent: &Agent<M>,
        prompt: Message,
    ) -> Result<&AnalysisResponse> {
        let index = self.next_batch_index();
        if index >= self.checkpoint.total_batches {
            return Err(Error::AnalysisComplete {
                total: self.checkpoint.total_batches,
            });
        }

        let response = agent.analyze(prompt).await?;
        self.checkpoint.responses.push(response);
        if let Err(error) = save_checkpoint(&self.checkpoint, &self.progress_path) {
            self.checkpoint.responses.pop();
            return Err(error);
        }
        tracing::info!(
            session_id = %self.session.id,
            completed_batches = self.checkpoint.responses.len(),
            total_batches = self.checkpoint.total_batches,
            "analysis checkpoint saved"
        );
        if self.checkpoint.responses.len() == self.checkpoint.total_batches {
            tracing::info!(
                session_id = %self.session.id,
                total_batches = self.checkpoint.total_batches,
                "analysis complete"
            );
        }

        Ok(self
            .checkpoint
            .responses
            .last()
            .expect("the completed response was just appended"))
    }

    async fn materialize_prompt(&self, batch: &[FrameSet]) -> Result<Message> {
        let mut content = prompt_content(&self.checklist, self.checkpoint.responses.last())?;

        for frame_set in batch {
            let timestamp = format_timestamp(frame_set.session_offset);
            append_prompt_frame_set(&mut content, &timestamp);
            for frame in &frame_set.frames {
                let camera = self
                    .session
                    .cameras
                    .iter()
                    .find(|camera| camera.id == frame.camera_id)
                    .ok_or(Error::MissingCamera {
                        camera_id: frame.camera_id,
                    })?;
                let path = frame.path.clone();
                let offset = frame.recording_offset;
                let jpeg =
                    tokio::task::spawn_blocking(move || extract_jpeg(&path, offset)).await??;
                append_prompt_frame(&mut content, camera.id, &camera.name, &timestamp, &jpeg);
                drop(jpeg);
            }
        }

        Ok(Message::User { content })
    }
}

fn load_or_new(
    path: &Path,
    session_id: uuid::Uuid,
    checklist: &str,
    plan_fingerprint: &str,
    total_batches: usize,
    warnings: &[AnalysisWarning],
) -> Result<(AnalysisCheckpoint, bool)> {
    match AnalysisCheckpoint::read(path, session_id) {
        Ok(checkpoint) => {
            if checkpoint.checklist != checklist {
                return Err(Error::CheckpointChecklist);
            }
            if checkpoint.plan_fingerprint != plan_fingerprint {
                return Err(Error::CheckpointPlanFingerprint);
            }
            if checkpoint.total_batches != total_batches {
                return Err(Error::CheckpointBatchCount {
                    expected: total_batches,
                    actual: checkpoint.total_batches,
                });
            }
            if checkpoint.warnings != warnings {
                return Err(Error::CheckpointWarnings);
            }
            Ok((checkpoint, false))
        }
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            if checklist.is_empty() {
                return Err(Error::EmptyCheckpointChecklist);
            }
            Ok((
                AnalysisCheckpoint {
                    schema_version: ANALYSIS_SCHEMA_VERSION,
                    session_id,
                    checklist: checklist.to_owned(),
                    plan_fingerprint: plan_fingerprint.to_owned(),
                    total_batches,
                    warnings: warnings.to_vec(),
                    responses: Vec::new(),
                },
                true,
            ))
        }
        Err(error) => Err(error),
    }
}

fn save_checkpoint(checkpoint: &AnalysisCheckpoint, path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), checkpoint)?;
    temporary.as_file_mut().write_all(b"\n")?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn plan_fingerprint(frame_sets: &[FrameSet], frame_sets_per_batch: NonZeroUsize) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"leo-analysis-plan-v2\0");
    hash.update(
        u64::try_from(frame_sets_per_batch.get())
            .map_err(|_| Error::PlanValueOverflow {
                field: "batch size",
            })?
            .to_le_bytes(),
    );
    hash.update(
        u64::try_from(frame_sets.len())
            .map_err(|_| Error::PlanValueOverflow {
                field: "frame-set count",
            })?
            .to_le_bytes(),
    );

    for frame_set in frame_sets {
        hash.update(
            u64::try_from(frame_set.session_offset.as_millis())
                .map_err(|_| Error::PlanValueOverflow {
                    field: "frame-set offset",
                })?
                .to_le_bytes(),
        );
        hash.update(
            u64::try_from(frame_set.frames.len())
                .map_err(|_| Error::PlanValueOverflow {
                    field: "frame count",
                })?
                .to_le_bytes(),
        );
        for frame in &frame_set.frames {
            hash.update(frame.camera_id.to_le_bytes());
            hash.update(frame.segment_start_utc_ms.to_le_bytes());
            hash.update(frame.segment_end_utc_ms.to_le_bytes());
            hash.update(
                u64::try_from(frame.sample_index)
                    .map_err(|_| Error::PlanValueOverflow {
                        field: "sample index",
                    })?
                    .to_le_bytes(),
            );
            hash.update(
                u64::try_from(frame.recording_offset.as_millis())
                    .map_err(|_| Error::PlanValueOverflow {
                        field: "recording offset",
                    })?
                    .to_le_bytes(),
            );
        }
    }

    Ok(format!("{:x}", hash.finalize()))
}

fn format_timestamp(offset: Duration) -> String {
    let total_millis = offset.as_millis();
    let total_seconds = total_millis / 1_000;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds / 60 % 60;
    let seconds = total_seconds % 60;
    let millis = total_millis % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn prompt_content(
    checklist: &str,
    previous: Option<&AnalysisResponse>,
) -> Result<OneOrMany<UserContent>> {
    let previous = previous
        .map(serde_json::to_string)
        .transpose()?
        .map(|response| format!("Previous complete analysis response:\n{response}"))
        .unwrap_or_else(|| "This is the first batch; there is no previous response.".into());
    let mut content = OneOrMany::one(UserContent::text(format!(
        "Correct sequence checklist:\n{checklist}"
    )));
    content.push(UserContent::text(previous));
    Ok(content)
}

fn append_prompt_frame_set(content: &mut OneOrMany<UserContent>, timestamp: &str) {
    content.push(UserContent::text(format!(
        "Frame set timestamp: {timestamp}"
    )));
}

fn append_prompt_frame(
    content: &mut OneOrMany<UserContent>,
    camera_id: u32,
    camera_name: &str,
    timestamp: &str,
    jpeg: &[u8],
) {
    content.push(UserContent::text(format!(
        "Frame source: camera {camera_id} ({camera_name}) at {timestamp}"
    )));
    content.push(UserContent::image_base64(
        STANDARD.encode(jpeg),
        Some(ImageMediaType::JPEG),
        None,
    ));
}

#[cfg(test)]
mod tests {
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
        let mut content = prompt_content("Open the valve", Some(&previous))
            .expect("prompt header should be built");

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
        let extraction_directory =
            tempfile::tempdir().expect("temporary directory should be created");
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

        let provider_directory =
            tempfile::tempdir().expect("temporary directory should be created");
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
    async fn resume_rejects_changed_checklist_plan_or_warnings() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let exercise = || session(vec![camera(1, true, 2), camera(2, false, 1)]);
        Analyzer::resume(
            vec![covering_segment(1)],
            exercise(),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            checkpoint.clone(),
        )
        .await
        .expect("initial plan should be checkpointed");

        let changed_checklist = Analyzer::resume(
            vec![covering_segment(1)],
            exercise(),
            "Use a different checklist".into(),
            NonZeroUsize::new(2).unwrap(),
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
            checkpoint.clone(),
        )
        .await;
        assert!(matches!(
            changed_plan,
            Err(super::Error::CheckpointPlanFingerprint)
        ));

        let changed_warnings = Analyzer::resume(
            vec![covering_segment(1), covering_segment(2)],
            exercise(),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
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
        )
        .expect("first plan should be fingerprinted");
        let second = plan_fingerprint(
            &fingerprint_plan(PathBuf::from("/different/location/segment.mkv")),
            NonZeroUsize::new(5).unwrap(),
        )
        .expect("second plan should be fingerprinted");

        assert_eq!(first, second);
    }

    #[test]
    fn fingerprint_encoding_is_stable() {
        let fingerprint = plan_fingerprint(
            &fingerprint_plan(PathBuf::from("/excluded/from/fingerprint.mkv")),
            NonZeroUsize::new(5).unwrap(),
        )
        .expect("golden plan should be fingerprinted");

        assert_eq!(
            fingerprint,
            "2e61898616fe0b02dda021e2bc83131bd38ec7e2fb1681f051934ee9a3ef288a"
        );
    }

    #[tokio::test]
    async fn resume_rebuilds_the_canonical_plan_and_fixed_batches() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let analyzer = Analyzer::resume(
            vec![covering_segment(1)],
            session(vec![camera(1, true, 2), camera(2, false, 1)]),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
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
        assert_eq!(analyzer.frame_sets.chunks(2).count(), 2);
        assert_eq!(analyzer.checkpoint.total_batches, 2);
        assert_eq!(analyzer.next_batch_index(), 0);
        assert_eq!(
            AnalysisCheckpoint::read(&checkpoint, Uuid::from_u128(1))
                .expect("initial checkpoint should be readable"),
            analyzer.checkpoint
        );
    }

    #[tokio::test]
    async fn resume_rejects_an_empty_merged_plan() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let result = Analyzer::resume(
            Vec::new(),
            session(vec![camera(1, true, 1)]),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            directory.path().join("analysis.json"),
        )
        .await;

        assert!(matches!(result, Err(super::Error::NoAnalyzableFrames)));
    }
}
