//! Sequential, resumable analysis. Each frame window is scanned once for the whole checklist.
use super::{RunError, RunResult, RunStore, jobs, model::*, recheck, results, review};
use crate::{
    analysis::{build_evidence, plan_windows, verify_sources},
    exercise::{ExerciseRevision, ExerciseStore},
    model::{ExecutionPolicy, ModelSnapshot, ProviderConnection, ScheduledJob, plan_execution},
    recording::list_segments,
    session::Session,
};
use std::{fs, path::PathBuf};

/// Operator-selected immutable exercise and one model; additions are explicit evidence options.
pub struct CreateRun {
    pub session_directory: PathBuf,
    /// Session identity selected by the operator, rechecked when its directory is read.
    pub expected_session_id: uuid::Uuid,
    pub exercise_store: ExerciseStore,
    pub revision: ExerciseRevision,
    pub model: ModelSnapshot,
    pub execution: ExecutionPolicy,
    pub evidence_policy: EvidencePolicy,
}
/// Current credentials are provided for execution, never written into saved run inputs.
pub struct ExecuteRun {
    pub session_directory: PathBuf,
    pub run_id: uuid::Uuid,
    pub providers: Vec<ProviderConnection>,
}

/// Prepare an immutable run for the selected recording identity on a filesystem worker.
pub async fn create_run(request: CreateRun) -> RunResult<RunSnapshot> {
    tokio::task::spawn_blocking(move || {
        request.execution.validate()?;
        request.model.profile.validate()?;
        if !fs::symlink_metadata(&request.session_directory)
            .is_ok_and(|metadata| metadata.file_type().is_dir())
            || !fs::symlink_metadata(request.session_directory.join("recording-complete"))
                .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
        {
            return Err(RunError::Invalid("Select a completed recording".into()));
        }
        plan_execution(
            &[ScheduledJob {
                id: "scan".into(),
                profile_id: request.model.profile.id.clone(),
            }],
            std::slice::from_ref(&request.model),
            &request.execution,
        )?;
        let session = Session::load(&request.session_directory.join("events.jsonl"))
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        if session.id != request.expected_session_id {
            return Err(RunError::Invalid(
                "The selected recording changed on disk. Refresh sessions and select it again."
                    .into(),
            ));
        }
        let cameras = session
            .cameras
            .iter()
            .map(|camera| camera.id)
            .collect::<Vec<_>>();
        let segments = list_segments(&request.session_directory.join("recordings"), &cameras)
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        let evidence = build_evidence(&request.session_directory, &session, &segments)?;
        let references = if request.evidence_policy.reference_images {
            request.revision.references.len()
        } else {
            0
        };
        let windows = plan_windows(
            &evidence,
            request.model.profile.image_limit,
            references,
            request.evidence_policy.max_window_span_ms,
            request.evidence_policy.overlap_frame_sets,
        )?;
        if windows.is_empty() {
            return Err(RunError::Invalid(
                "This recording has no eligible candidate frames".into(),
            ));
        }
        let inputs = RunInputs {
            schema_version: 2,
            id: uuid::Uuid::new_v4(),
            created_utc_ms: review::now()?,
            prompt_version: jobs::PROMPT_VERSION.into(),
            exercise: request.revision,
            model: request.model,
            execution: request.execution,
            evidence_policy: request.evidence_policy,
            initial_evidence: evidence.clone(),
            windows,
        };
        let jobs = inputs
            .windows
            .iter()
            .map(|window| jobs::record(&inputs, JobKind::Scan, None, window.frame_ids.clone()))
            .collect::<RunResult<Vec<_>>>()?;
        let checkpoint = RunCheckpoint {
            stage: RunStage::Preparing,
            jobs,
            evidence,
            assessments: Vec::new(),
            error: None,
        };
        let snapshot = RunSnapshot {
            inputs,
            checkpoint,
            responses: Vec::new(),
            corrections: Vec::new(),
            operator_review: OperatorReview::default(),
        };
        RunStore::new(request.session_directory).create(
            &snapshot.inputs,
            &snapshot.checkpoint,
            &request.exercise_store,
        )?;
        Ok(snapshot)
    })
    .await?
}

/// Explicitly start or resume. No provider failure is retried within this invocation.
pub async fn execute_run(
    request: ExecuteRun,
    mut on_checkpoint: impl FnMut(RunSnapshot),
) -> RunResult<RunSnapshot> {
    let store = RunStore::new(&request.session_directory);
    let startup = store.clone();
    let id = request.run_id;
    let (_lock, mut snapshot) = tokio::task::spawn_blocking(move || {
        let lock = startup.execution_lock(id)?;
        Ok::<_, RunError>((lock, startup.load(id)?))
    })
    .await??;
    if snapshot.checkpoint.stage == RunStage::Ready {
        on_checkpoint(snapshot.clone());
        return Ok(snapshot);
    }
    for job in &mut snapshot.checkpoint.jobs {
        if job.state != JobState::Complete {
            job.state = JobState::Pending;
        }
    }
    snapshot.checkpoint.error = None;
    let result = stages(
        &store,
        &request.providers,
        &mut snapshot,
        &mut on_checkpoint,
    )
    .await;
    if let Err(error) = result {
        snapshot.checkpoint.stage = RunStage::Paused;
        snapshot.checkpoint.error = Some(error.to_string());
        publish(&store, &snapshot, &mut on_checkpoint).await?;
        return Err(error);
    }
    Ok(snapshot)
}

/// Reserve the one closer look before starting it. Competing requests cannot spend it twice.
pub async fn request_recheck(
    directory: PathBuf,
    run_id: uuid::Uuid,
    criterion_id: uuid::Uuid,
    frame_id: Option<String>,
) -> RunResult<RunSnapshot> {
    tokio::task::spawn_blocking(move || {
        let store = RunStore::new(directory);
        let _lock = store.execution_lock(run_id)?;
        let mut snapshot = store.load(run_id)?;
        if snapshot.checkpoint.stage != RunStage::Ready {
            return Err(RunError::Invalid(
                "Finish or resume the current analysis first".into(),
            ));
        }
        recheck::queue(&mut snapshot, criterion_id, frame_id.as_deref())?;
        store.save_checkpoint(run_id, &snapshot.checkpoint)?;
        Ok(snapshot)
    })
    .await?
}

async fn stages(
    store: &RunStore,
    providers: &[ProviderConnection],
    snapshot: &mut RunSnapshot,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<()> {
    let directory = store.session_directory.clone();
    let evidence = snapshot.checkpoint.evidence.clone();
    tokio::task::spawn_blocking(move || verify_sources(&directory, &evidence)).await??;
    run_pending(store, providers, snapshot, JobKind::Scan, callback).await?;
    if !snapshot
        .checkpoint
        .jobs
        .iter()
        .any(|job| job.kind == JobKind::Consolidate)
    {
        let mut frames = jobs::observations(snapshot)
            .iter()
            .flat_map(|observation| observation.evidence_ids.clone())
            .collect::<Vec<_>>();
        frames.sort();
        frames.dedup();
        snapshot.checkpoint.jobs.push(jobs::record(
            &snapshot.inputs,
            JobKind::Consolidate,
            None,
            frames,
        )?);
        publish(store, snapshot, callback).await?;
    }
    run_pending(store, providers, snapshot, JobKind::Consolidate, callback).await?;
    results::assess(snapshot);
    recheck::queue_automatic(snapshot);
    publish(store, snapshot, callback).await?;
    run_pending(store, providers, snapshot, JobKind::Recheck, callback).await?;
    results::assess(snapshot);
    snapshot.checkpoint.stage = RunStage::Ready;
    publish(store, snapshot, callback).await
}

async fn run_pending(
    store: &RunStore,
    providers: &[ProviderConnection],
    snapshot: &mut RunSnapshot,
    kind: JobKind,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<()> {
    let pending = snapshot
        .checkpoint
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.kind == kind && job.state == JobState::Pending)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    for index in pending {
        snapshot.checkpoint.stage = match kind {
            JobKind::Scan => RunStage::Scanning,
            JobKind::Consolidate => RunStage::Consolidating,
            JobKind::Recheck => RunStage::Rechecking,
        };
        snapshot.checkpoint.jobs[index].state = JobState::Running;
        publish(store, snapshot, callback).await?;
        let job = snapshot.checkpoint.jobs[index].clone();
        let result = execute_job(store, providers, snapshot, &job).await;
        match result {
            Ok(response) => {
                snapshot.responses.push(response);
                snapshot.checkpoint.jobs[index].state = JobState::Complete;
            }
            Err(error) => {
                let message = error.to_string();
                snapshot.checkpoint.jobs[index].state = JobState::Failed {
                    message: message.clone(),
                };
                snapshot.checkpoint.jobs[index].failures.push(message);
                return Err(error);
            }
        }
        publish(store, snapshot, callback).await?;
    }
    Ok(())
}
async fn execute_job(
    store: &RunStore,
    providers: &[ProviderConnection],
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<SavedResponse> {
    let recovery = store.clone();
    let state = snapshot.clone();
    let request = job.clone();
    if let Some(response) =
        tokio::task::spawn_blocking(move || recovery.saved_response(&state, &request)).await??
    {
        return Ok(response);
    }
    let matching = providers
        .iter()
        .filter(|provider| provider.id == snapshot.inputs.model.provider.id)
        .collect::<Vec<_>>();
    let [provider] = matching.as_slice() else {
        return Err(RunError::Invalid(
            "Selected provider is missing or duplicated".into(),
        ));
    };
    let preparation_store = store.clone();
    let state = snapshot.clone();
    let request = job.clone();
    let prepared =
        tokio::task::spawn_blocking(move || jobs::prepare(&preparation_store, &state, &request))
            .await??;
    let response = jobs::complete(snapshot, job, provider, prepared).await?;
    let storage = store.clone();
    let state = snapshot.clone();
    let request = job.clone();
    tokio::task::spawn_blocking(move || {
        storage.save_response(&state, &request, &response)?;
        Ok(response)
    })
    .await?
}
async fn publish(
    store: &RunStore,
    snapshot: &RunSnapshot,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<()> {
    let storage = store.clone();
    let id = snapshot.inputs.id;
    let checkpoint = snapshot.checkpoint.clone();
    tokio::task::spawn_blocking(move || storage.save_checkpoint(id, &checkpoint)).await??;
    callback(snapshot.clone());
    Ok(())
}
