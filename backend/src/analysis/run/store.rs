//! Atomic local run persistence. Saved responses are the source of truth after interruption.
use super::{RunError, RunResult, legacy::ArchivedRun, model::*};
use crate::exercise::ExerciseStore;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// History tolerates one damaged or older run without hiding other recordings' results.
#[derive(Debug, Clone, Default)]
pub struct RunHistory {
    pub runs: Vec<RunSnapshot>,
    pub archived: Vec<ArchivedRun>,
}

/// Blocking store under a single session's analyses directory. UI callers use a worker.
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

    /// Freeze an already-published exercise and verified reference originals atomically.
    pub fn create(
        &self,
        inputs: &RunInputs,
        checkpoint: &RunCheckpoint,
        exercises: &ExerciseStore,
    ) -> RunResult<()> {
        if exercises.load_revision(inputs.exercise.exercise_id, inputs.exercise.id)?
            != inputs.exercise
        {
            return Err(RunError::Invalid(
                "Exercise is not a saved operator revision".into(),
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

    pub fn history(&self) -> RunResult<RunHistory> {
        let root = self.session_directory.join("analyses");
        let mut history = RunHistory::default();
        if !root.try_exists()? {
            return Ok(history);
        }
        require_directory(&root)?;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            match self.load(id) {
                Ok(run) => history.runs.push(run),
                Err(error) => {
                    history
                        .archived
                        .push(super::legacy::read(self, id, error.to_string()))
                }
            }
        }
        history
            .runs
            .sort_by_key(|run| std::cmp::Reverse((run.inputs.created_utc_ms, run.inputs.id)));
        Ok(history)
    }

    /// Recover complete requests from response files even when the last checkpoint was interrupted.
    pub fn load(&self, id: Uuid) -> RunResult<RunSnapshot> {
        let directory = self.directory(id)?;
        let inputs: RunInputs = read_json(&directory.join("inputs.json"))?;
        let checkpoint: RunCheckpoint = read_json(&directory.join("checkpoint.json"))?;
        if inputs.schema_version != 2
            || inputs.id != id
            || checkpoint.evidence.session_id != inputs.initial_evidence.session_id
        {
            return Err(RunError::Invalid("Run identity or schema mismatch".into()));
        }
        let corrections = self.read_optional(id, "review.json")?;
        let operator_review = self.read_optional(id, "operator-review.json")?;
        let mut snapshot = RunSnapshot {
            inputs,
            checkpoint,
            responses: Vec::new(),
            corrections,
            operator_review,
        };
        let mut ids = std::collections::HashSet::new();
        let mut rechecks = std::collections::HashSet::new();
        for job in &mut snapshot.checkpoint.jobs {
            if !ids.insert(&job.id)
                || (job.kind == JobKind::Recheck && !rechecks.insert(job.criterion_id))
            {
                return Err(RunError::Invalid("Repeated analysis request".into()));
            }
            let path = self.response_path(id, &job.id)?;
            if path.try_exists()? {
                snapshot.responses.push(read_json(&path)?);
                job.state = JobState::Complete;
            } else if job.state == JobState::Complete {
                return Err(RunError::Invalid(
                    "Completed request is missing its response".into(),
                ));
            }
        }
        for response in &snapshot.responses {
            let job = snapshot
                .checkpoint
                .jobs
                .iter()
                .find(|job| job.id == response.job_id)
                .ok_or_else(|| RunError::Invalid("Unrecognized saved response".into()))?;
            super::results::validate_response(&snapshot, job, response)?;
        }
        super::results::assess(&mut snapshot);
        Ok(snapshot)
    }

    pub fn save_checkpoint(&self, id: Uuid, checkpoint: &RunCheckpoint) -> RunResult<()> {
        write_json(
            &self.directory(id)?.join("checkpoint.json"),
            checkpoint,
            true,
        )
    }
    /// Recover a response whose publication preceded a crashed checkpoint update.
    pub fn saved_response(
        &self,
        snapshot: &RunSnapshot,
        job: &JobRecord,
    ) -> RunResult<Option<SavedResponse>> {
        let path = self.response_path(snapshot.inputs.id, &job.id)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let response = read_json(&path)?;
        super::results::validate_response(snapshot, job, &response)?;
        Ok(Some(response))
    }
    pub fn save_response(
        &self,
        snapshot: &RunSnapshot,
        job: &JobRecord,
        response: &SavedResponse,
    ) -> RunResult<()> {
        super::results::validate_response(snapshot, job, response)?;
        write_json(
            &self.response_path(snapshot.inputs.id, &job.id)?,
            response,
            false,
        )
    }
    pub fn execution_lock(&self, id: Uuid) -> RunResult<File> {
        lock_file(&self.directory(id)?.join(".execution-lock"))
    }
    pub fn review_lock(&self, id: Uuid) -> RunResult<File> {
        lock_file(&self.directory(id)?.join(".review-lock"))
    }
    pub fn directory(&self, id: Uuid) -> RunResult<PathBuf> {
        require_directory(&self.session_directory)?;
        let root = self.session_directory.join("analyses");
        require_directory(&root)?;
        let directory = root.join(id.to_string());
        require_directory(&directory)?;
        Ok(directory)
    }
    pub fn read_reference(&self, snapshot: &RunSnapshot, asset_id: Uuid) -> RunResult<Vec<u8>> {
        let asset = snapshot
            .inputs
            .exercise
            .references
            .iter()
            .find(|reference| reference.image.id == asset_id)
            .map(|reference| &reference.image)
            .ok_or_else(|| RunError::Invalid("Unknown reference image".into()))?;
        let path = self
            .directory(snapshot.inputs.id)?
            .join("references")
            .join(format!("{}.{}", asset.id, asset.media_type.extension()));
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(RunError::Invalid("Expected a reference original".into()));
        }
        let bytes = fs::read(path)?;
        if bytes.len() as u64 != asset.byte_length || hash(&bytes) != asset.sha256 {
            return Err(RunError::Invalid("Reference original changed".into()));
        }
        Ok(bytes)
    }
    fn response_path(&self, id: Uuid, job_id: &str) -> RunResult<PathBuf> {
        if job_id.len() != 64 || !job_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RunError::Invalid("Invalid request identity".into()));
        }
        Ok(self
            .directory(id)?
            .join("responses")
            .join(format!("{job_id}.json")))
    }
    fn read_optional<T: DeserializeOwned + Default>(&self, id: Uuid, name: &str) -> RunResult<T> {
        let path = self.directory(id)?.join(name);
        if path.try_exists()? {
            read_json(&path)
        } else {
            Ok(T::default())
        }
    }
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
