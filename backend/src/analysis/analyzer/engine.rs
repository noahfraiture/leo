use std::{
    io::Write,
    num::NonZeroUsize,
    ops::Range,
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
    overlap_frame_sets: usize,
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
        overlap_frame_sets: usize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        let session_id = session.id;
        tracing::info!(session_id = %session_id, "planning analysis");

        Self::resume_inner(
            segments,
            session,
            checklist,
            frame_sets_per_batch,
            overlap_frame_sets,
            progress_path,
        )
        .inspect_err(|_| tracing::error!(session_id = %session_id, "analysis planning failed"))
    }

    fn resume_inner(
        segments: Vec<RecordingSegment>,
        session: Session,
        checklist: String,
        frame_sets_per_batch: NonZeroUsize,
        overlap_frame_sets: usize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        if overlap_frame_sets >= frame_sets_per_batch.get() {
            return Err(Error::InvalidBatchOverlap);
        }
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
        let plan_fingerprint =
            plan_fingerprint(&frame_sets, frame_sets_per_batch, overlap_frame_sets)?;
        let stride = frame_sets_per_batch.get() - overlap_frame_sets;
        let total_batches = frame_sets
            .len()
            .saturating_sub(frame_sets_per_batch.get())
            .div_ceil(stride)
            + 1;
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
            overlap_frame_sets,
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

        let range = self.batch_range(index);
        let prompt = match self.materialize_prompt(&self.frame_sets[range]).await {
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

    fn batch_range(&self, index: usize) -> Range<usize> {
        debug_assert!(index < self.checkpoint.total_batches);
        let stride = self.frame_sets_per_batch.get() - self.overlap_frame_sets;
        let start = index * stride;
        start
            ..start
                .saturating_add(self.frame_sets_per_batch.get())
                .min(self.frame_sets.len())
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

fn plan_fingerprint(
    frame_sets: &[FrameSet],
    frame_sets_per_batch: NonZeroUsize,
    overlap_frame_sets: usize,
) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"leo-analysis-plan-v3\0");
    hash.update(
        u64::try_from(frame_sets_per_batch.get())
            .map_err(|_| Error::PlanValueOverflow {
                field: "batch size",
            })?
            .to_le_bytes(),
    );
    hash.update(
        u64::try_from(overlap_frame_sets)
            .map_err(|_| Error::PlanValueOverflow {
                field: "batch overlap",
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
#[path = "tests/engine.rs"]
mod tests;
