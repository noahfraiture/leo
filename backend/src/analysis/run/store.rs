use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::{
    SentImage,
    reconcile::{EvidenceSelection, FinalFinding, ReaderFinding, ReviewFinding},
};
use super::model::{FinalResult, ReaderResult, ReviewResult, SelectionResult};
use super::{
    HumanCorrection, JobKind, JobRecord, JobState, RunCheckpoint, RunError, RunInputs, RunResult,
    RunSnapshot,
};
use crate::{exercise::ExerciseStore, model::Completion};

/// Original typed response and the identities of the actual images sent with that request.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedResponse {
    pub job_id: String,
    pub frame_ids: Vec<String>,
    pub sent_images: Vec<SentImage>,
    pub reference_hashes: Vec<(Uuid, String)>,
    pub output: JobOutput,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum JobOutput {
    Reader(Completion<ReaderFinding>),
    Review(Completion<ReviewFinding>),
    Selection(Completion<EvidenceSelection>),
    Final(Completion<FinalFinding>),
}

/// Blocking store under a single session's `analyses` directory. Call from a worker in UI code.
#[derive(Debug, Clone)]
pub struct RunStore {
    pub session_directory: PathBuf,
}

impl RunStore {
    pub fn new(session_directory: impl Into<PathBuf>) -> Self {
        Self {
            session_directory: session_directory.into(),
        }
    }

    /// Publishes complete immutable inputs and copied reference originals in one directory rename.
    pub fn create(
        &self,
        inputs: &RunInputs,
        checkpoint: &RunCheckpoint,
        exercises: &ExerciseStore,
    ) -> RunResult<()> {
        let original = exercises.load_revision(inputs.exercise.exercise_id, inputs.exercise.id)?;
        if original != inputs.exercise {
            return Err(RunError::Invalid(
                "Exercise snapshot is not a saved operator revision".into(),
            ));
        }
        let root = self.session_directory.join("analyses");
        ensure_directory(&root)?;
        let temporary = tempfile::Builder::new()
            .prefix(".preparing-")
            .tempdir_in(&root)?;
        for reference in &inputs.exercise.references {
            let bytes = exercises.read_reference(inputs.exercise.exercise_id, &reference.image)?;
            write_bytes(
                &temporary.path().join("references").join(format!(
                    "{}.{}",
                    reference.image.id,
                    reference.image.media_type.extension()
                )),
                &bytes,
                false,
            )?;
        }
        write_json(&temporary.path().join("inputs.json"), inputs, false)?;
        write_json(&temporary.path().join("checkpoint.json"), checkpoint, false)?;
        ensure_directory(&temporary.path().join("responses"))?;
        ensure_directory(&temporary.path().join("cache"))?;
        fs::rename(temporary.path(), root.join(inputs.id.to_string()))?;
        File::open(root)?.sync_all()?;
        Ok(())
    }

    /// Enumerates only fully published runs, newest first, reporting corrupt data explicitly.
    pub fn list(&self) -> RunResult<Vec<RunSnapshot>> {
        let root = self.session_directory.join("analyses");
        if !root.try_exists()? {
            return Ok(Vec::new());
        }
        require_directory(&root)?;
        let mut runs = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| Uuid::parse_str(name).ok())
            {
                runs.push(self.load(id)?);
            }
        }
        runs.sort_by_key(|run| std::cmp::Reverse((run.inputs.created_utc_ms, run.inputs.id)));
        Ok(runs)
    }

    /// Reconstructs progress from durable responses even if their checkpoint update was interrupted.
    pub fn load(&self, id: Uuid) -> RunResult<RunSnapshot> {
        let directory = self.directory(id)?;
        let inputs: RunInputs = read_json(&directory.join("inputs.json"))?;
        let mut checkpoint: RunCheckpoint = read_json(&directory.join("checkpoint.json"))?;
        if inputs.schema_version != 1
            || inputs.id != id
            || checkpoint.evidence.session_id != inputs.initial_evidence.session_id
        {
            return Err(RunError::Invalid("Run identity or schema mismatch".into()));
        }
        let mut reader_results = Vec::new();
        let mut review_results = Vec::new();
        let mut final_results = Vec::new();
        let mut selection_results = Vec::new();
        for job in &mut checkpoint.jobs {
            if let Some(response) = self.response(id, job)? {
                match response.output {
                    JobOutput::Reader(completion) => reader_results.push(ReaderResult {
                        job_id: job.id.clone(),
                        completion,
                    }),
                    JobOutput::Review(completion) => review_results.push(ReviewResult {
                        job_id: job.id.clone(),
                        completion,
                    }),
                    JobOutput::Final(completion) => final_results.push(FinalResult {
                        job_id: job.id.clone(),
                        completion,
                    }),
                    JobOutput::Selection(completion) => selection_results.push(SelectionResult {
                        job_id: job.id.clone(),
                        completion,
                    }),
                }
                job.state = JobState::Complete;
            } else if job.state == JobState::Complete {
                return Err(RunError::Invalid(
                    "A completed job has no saved response".into(),
                ));
            }
        }
        let corrections = if directory.join("review.json").try_exists()? {
            read_json(&directory.join("review.json"))?
        } else {
            Vec::new()
        };
        Ok(RunSnapshot {
            inputs,
            checkpoint,
            reader_results,
            review_results,
            final_results,
            selection_results,
            corrections,
        })
    }

    /// Appends a correction without changing model results, references, or future prompts.
    pub fn append_human_correction(&self, id: Uuid, correction: HumanCorrection) -> RunResult<()> {
        let directory = self.directory(id)?;
        let _lock = lock_file(&directory.join(".review-lock"))?;
        let mut snapshot = self.load(id)?;
        if correction.id.is_nil()
            || correction.reason.trim().is_empty()
            || !snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .iter()
                .any(|criterion| criterion.id == correction.criterion_id)
            || correction
                .evidence_ids
                .iter()
                .any(|evidence| snapshot.checkpoint.evidence.frame(evidence).is_none())
            || snapshot
                .corrections
                .iter()
                .any(|saved| saved.id == correction.id)
        {
            return Err(RunError::Invalid(
                "Invalid correction identity, criterion, reason, or evidence".into(),
            ));
        }
        let previous = snapshot
            .corrections
            .iter()
            .rev()
            .find(|saved| saved.criterion_id == correction.criterion_id)
            .map(|saved| saved.id);
        if correction.supersedes != previous {
            return Err(RunError::Invalid(
                "Correction must supersede the latest correction for this criterion".into(),
            ));
        }
        snapshot.corrections.push(correction);
        write_json(&directory.join("review.json"), &snapshot.corrections, true)
    }

    pub fn directory(&self, id: Uuid) -> RunResult<PathBuf> {
        let root = self.session_directory.join("analyses");
        require_directory(&root)?;
        let directory = root.join(id.to_string());
        require_directory(&directory)?;
        Ok(directory)
    }

    /// Reads a frozen original and checks its saved byte identity before playback or preview.
    pub fn read_reference(
        &self,
        id: Uuid,
        asset: &crate::exercise::ImageAsset,
    ) -> RunResult<Vec<u8>> {
        let inputs: RunInputs = read_json(&self.directory(id)?.join("inputs.json"))?;
        if !inputs
            .exercise
            .references
            .iter()
            .any(|reference| reference.image == *asset)
        {
            return Err(RunError::Invalid(
                "Reference does not belong to this run".into(),
            ));
        }
        let bytes = fs::read(self.directory(id)?.join("references").join(format!(
            "{}.{}",
            asset.id,
            asset.media_type.extension()
        )))?;
        if bytes.len() as u64 != asset.byte_length || hash(&bytes) != asset.sha256 {
            return Err(RunError::Invalid(
                "Copied reference original changed".into(),
            ));
        }
        Ok(bytes)
    }

    /// Holds an OS-managed lock for the lifetime of an executor; crashes release it automatically.
    pub fn execution_lock(&self, id: Uuid) -> RunResult<File> {
        lock_file(&self.directory(id)?.join(".execution-lock"))
    }

    pub fn save_checkpoint(&self, id: Uuid, checkpoint: &RunCheckpoint) -> RunResult<()> {
        write_json(
            &self.directory(id)?.join("checkpoint.json"),
            checkpoint,
            true,
        )
    }

    /// Accepts an output before checkpoint mutation. An existing accepted response is never replaced.
    pub fn save_response(
        &self,
        id: Uuid,
        job: &JobRecord,
        response: &SavedResponse,
    ) -> RunResult<()> {
        validate_response(job, response)?;
        self.validate_selection_sources(id, job, &response.output)?;
        write_json(&self.response_path(id, &job.id)?, response, false)
    }

    pub fn response(&self, id: Uuid, job: &JobRecord) -> RunResult<Option<SavedResponse>> {
        let path = self.response_path(id, &job.id)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let response = read_json(&path)?;
        validate_response(job, &response)?;
        self.validate_selection_sources(id, job, &response.output)?;
        Ok(Some(response))
    }

    // Review identities are durable inputs, so a selector cannot substitute an unrelated window's
    // real image for a contradiction. This checks provenance, not the semantic truth of relevance.
    fn validate_selection_sources(
        &self,
        id: Uuid,
        job: &JobRecord,
        output: &JobOutput,
    ) -> RunResult<()> {
        let JobOutput::Selection(selection) = output else {
            return Ok(());
        };
        let checkpoint: RunCheckpoint = read_json(&self.directory(id)?.join("checkpoint.json"))?;
        for disposition in &selection.value.reviews {
            let mut reviewed_ids = std::collections::HashSet::new();
            for review_id in &disposition.review_ids {
                let review = checkpoint
                    .jobs
                    .iter()
                    .find(|review| {
                        review.id == *review_id
                            && review.kind == JobKind::Review
                            && review.criterion_id == job.criterion_id
                    })
                    .ok_or_else(|| RunError::Invalid("Unknown source visual review".into()))?;
                reviewed_ids.extend(&review.frame_ids);
            }
            if disposition
                .decisive_ids
                .iter()
                .any(|id| !reviewed_ids.contains(id))
            {
                return Err(RunError::Invalid(
                    "Decisive evidence must belong to its named visual review windows".into(),
                ));
            }
        }
        Ok(())
    }

    fn response_path(&self, id: Uuid, job_id: &str) -> RunResult<PathBuf> {
        if job_id.len() != 64 || !job_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RunError::Invalid("Invalid job identity".into()));
        }
        Ok(self
            .directory(id)?
            .join("responses")
            .join(format!("{job_id}.json")))
    }
}

fn validate_response(job: &JobRecord, response: &SavedResponse) -> RunResult<()> {
    if response.job_id != job.id || response.frame_ids != job.frame_ids {
        return Err(RunError::Invalid(
            "Saved response input identity mismatch".into(),
        ));
    }
    if (job.kind != JobKind::SelectEvidence
        && response
            .sent_images
            .iter()
            .map(|image| &image.candidate_id)
            .collect::<Vec<_>>()
            != job.frame_ids.iter().collect::<Vec<_>>())
        || (job.kind == JobKind::SelectEvidence && !response.sent_images.is_empty())
        || response.sent_images.iter().any(|image| {
            image.size_bytes == 0
                || image.sha256.len() != 64
                || !image.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(RunError::Invalid(
            "Saved sent-image identity does not match the request".into(),
        ));
    }
    let matching_role = matches!(
        (&job.kind, &response.output),
        (JobKind::Reader, JobOutput::Reader(_))
            | (JobKind::Review, JobOutput::Review(_))
            | (JobKind::Final, JobOutput::Final(_))
            | (JobKind::SelectEvidence, JobOutput::Selection(_))
    );
    if !matching_role {
        return Err(RunError::Invalid("Saved response role mismatch".into()));
    }
    let result = match &response.output {
        JobOutput::Reader(value) => {
            super::super::reconcile::validate_reader(&value.value, &job.frame_ids)
        }
        JobOutput::Review(value) => {
            super::super::reconcile::validate_review(&value.value, &job.frame_ids)
        }
        JobOutput::Final(value) => {
            super::super::reconcile::validate_final(&value.value, &job.frame_ids)
        }
        JobOutput::Selection(value) => super::super::reconcile::validate_selection(
            &value.value,
            &job.frame_ids,
            &job.context_job_ids,
        ),
    };
    result.map_err(RunError::Invalid)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> RunResult<T> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(RunError::Invalid("Invalid analysis file".into()));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_json(path: &Path, value: &impl Serialize, replace: bool) -> RunResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes(path, &bytes, replace)
}

pub fn write_bytes(path: &Path, bytes: &[u8], replace: bool) -> RunResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RunError::Invalid("Invalid analysis path".into()))?;
    ensure_directory(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path).map_err(|error| error.error)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn ensure_directory(path: &Path) -> RunResult<()> {
    fs::create_dir_all(path)?;
    require_directory(path)
}

fn require_directory(path: &Path) -> RunResult<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(RunError::Invalid(
            "Expected a direct analysis directory".into(),
        ));
    }
    Ok(())
}

fn lock_file(path: &Path) -> RunResult<File> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.file_type().is_file()) {
        return Err(RunError::Invalid("Invalid analysis lock".into()));
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock().map_err(|_| RunError::Busy)?;
    Ok(file)
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
