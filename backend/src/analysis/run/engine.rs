use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::super::{build_evidence, plan_windows, verify_sources};
use super::store::{JobOutput, SavedResponse};
use super::{
    EvidencePolicy, JobKind, JobState, RunCheckpoint, RunError, RunInputs, RunResult, RunSnapshot,
    RunStage, RunStore, jobs, model::CriterionWindows, selection, workflow,
};
use crate::{
    exercise::{ExerciseRevision, ExerciseStore},
    model::{ExecutionPolicy, ProviderConnection, ResolvedModelSet, ScheduledJob, plan_execution},
    recording::list_segments,
    session::Session,
};

/// Inputs selected by the operator. Only an already published exercise revision may start a run.
pub struct CreateRun {
    pub session_directory: PathBuf,
    pub exercise_store: ExerciseStore,
    pub revision: ExerciseRevision,
    pub model_set: ResolvedModelSet,
    pub execution: ExecutionPolicy,
    pub evidence_policy: EvidencePolicy,
}

/// Credentials are supplied only for execution; saved runs retain their connection snapshots.
pub struct ExecuteRun {
    pub session_directory: PathBuf,
    pub run_id: uuid::Uuid,
    pub providers: Vec<ProviderConnection>,
}

/// Freezes recording identity, the one operator checklist, references, model set, and common windows.
pub async fn create_run(request: CreateRun) -> RunResult<RunSnapshot> {
    tokio::task::spawn_blocking(move || {
        request.execution.validate()?;
        if request.model_set.readers.is_empty() {
            return Err(RunError::Invalid(
                "Select at least one independent reader".into(),
            ));
        }
        if !fs::symlink_metadata(&request.session_directory)
            .is_ok_and(|metadata| metadata.file_type().is_dir())
            || !fs::symlink_metadata(request.session_directory.join("recording-complete"))
                .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
        {
            return Err(RunError::Invalid("Select a completed recording".into()));
        }
        let session = Session::load(&request.session_directory.join("events.jsonl"))
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        let camera_ids: Vec<_> = session.cameras.iter().map(|camera| camera.id).collect();
        let segments = list_segments(&request.session_directory.join("recordings"), &camera_ids)
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        let evidence = build_evidence(&request.session_directory, &session, &segments)?;
        let limit = request
            .model_set
            .readers
            .iter()
            .chain([&request.model_set.reviewer])
            .map(|model| model.profile.image_limit)
            .min()
            .unwrap();
        let windows = request
            .revision
            .checklist
            .criteria
            .iter()
            .map(|criterion| {
                let references = request
                    .revision
                    .references
                    .iter()
                    .filter(|reference| reference.criterion_ids.contains(&criterion.id))
                    .count();
                Ok(CriterionWindows {
                    criterion_id: criterion.id,
                    windows: plan_windows(
                        &evidence,
                        limit,
                        references,
                        request.evidence_policy.max_window_span_ms,
                        request.evidence_policy.overlap_frame_sets,
                    )?,
                })
            })
            .collect::<RunResult<Vec<_>>>()?;
        let inputs = RunInputs {
            schema_version: 1,
            id: uuid::Uuid::new_v4(),
            created_utc_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| RunError::Invalid("System clock precedes Unix time".into()))?
                .as_millis() as u64,
            prompt_version: "reference-independent-visual-review-v1".into(),
            exercise: request.revision,
            model_set: request.model_set,
            execution: request.execution,
            evidence_policy: request.evidence_policy,
            initial_evidence: evidence.clone(),
            windows,
        };
        let mut checkpoint = RunCheckpoint {
            stage: RunStage::Preparing,
            jobs: Vec::new(),
            evidence,
            gathered_criteria: Vec::new(),
            coverage_notes: Vec::new(),
            assessments: Vec::new(),
            error: None,
        };
        for criterion in &inputs.windows {
            for model in &inputs.model_set.readers {
                for window in &criterion.windows {
                    checkpoint.jobs.push(jobs::job_record(
                        &inputs,
                        criterion.criterion_id,
                        model,
                        JobKind::Reader,
                        0,
                        window.frame_ids.clone(),
                    )?);
                }
            }
        }
        let snapshot = RunSnapshot {
            inputs,
            checkpoint,
            reader_results: Vec::new(),
            review_results: Vec::new(),
            final_results: Vec::new(),
            selection_results: Vec::new(),
            corrections: Vec::new(),
        };
        let models = jobs::run_models(&snapshot)?;
        let ready: Vec<_> = snapshot
            .checkpoint
            .jobs
            .iter()
            .map(|job| ScheduledJob {
                id: job.id.clone(),
                profile_id: job.profile_id.clone(),
            })
            .collect();
        plan_execution(&ready, &models, &snapshot.inputs.execution)?;
        let model_checks: Vec<_> = models
            .iter()
            .enumerate()
            .map(|(index, model)| ScheduledJob {
                id: format!("preflight-{index}"),
                profile_id: model.profile.id.clone(),
            })
            .collect();
        plan_execution(&model_checks, &models, &snapshot.inputs.execution)?;
        let store = RunStore::new(request.session_directory);
        store.create(
            &snapshot.inputs,
            &snapshot.checkpoint,
            &request.exercise_store,
        )?;
        Ok(snapshot)
    })
    .await?
}

/// Runs or resumes independent jobs, emitting only durable progress. Failed jobs pause the run
/// after other independently schedulable work completes; an explicit subsequent call retries them.
pub async fn execute_run(
    request: ExecuteRun,
    mut on_checkpoint: impl FnMut(RunSnapshot),
) -> RunResult<RunSnapshot> {
    let store = RunStore::new(&request.session_directory);
    let startup_store = store.clone();
    let run_id = request.run_id;
    let (_lock, mut snapshot) = tokio::task::spawn_blocking(move || {
        let lock = startup_store.execution_lock(run_id)?;
        let snapshot = startup_store.load(run_id)?;
        Ok::<_, RunError>((lock, snapshot))
    })
    .await??;
    if snapshot.checkpoint.stage == RunStage::Ready {
        on_checkpoint(snapshot.clone());
        return Ok(snapshot);
    }
    for job in &mut snapshot.checkpoint.jobs {
        if matches!(job.state, JobState::Failed { .. } | JobState::Running) {
            job.state = JobState::Pending;
        }
    }
    snapshot.checkpoint.error = None;
    let result = execute_stages(
        &store,
        &request.providers,
        &mut snapshot,
        &mut on_checkpoint,
    )
    .await;
    if let Err(error) = result {
        let recovery_store = store.clone();
        if let Ok(Ok(recovered)) =
            tokio::task::spawn_blocking(move || recovery_store.load(run_id)).await
        {
            snapshot = recovered;
        }
        snapshot.checkpoint.stage = RunStage::Paused;
        snapshot.checkpoint.error = Some(error.to_string());
        publish(&store, &mut snapshot, &mut on_checkpoint).await?;
        return Err(error);
    }
    Ok(snapshot)
}

async fn execute_stages(
    store: &RunStore,
    providers: &[ProviderConnection],
    snapshot: &mut RunSnapshot,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<()> {
    let source_directory = store.session_directory.clone();
    let evidence = snapshot.checkpoint.evidence.clone();
    tokio::task::spawn_blocking(move || verify_sources(&source_directory, &evidence)).await??;
    if !run_pending(store, providers, snapshot, JobKind::Reader, callback).await? {
        return Ok(());
    }
    workflow::ensure_reviews(snapshot)?;
    if !run_pending(store, providers, snapshot, JobKind::Review, callback).await? {
        return Ok(());
    }
    snapshot.checkpoint.stage = RunStage::Gathering;
    workflow::gather(snapshot)?;
    publish(store, snapshot, callback).await?;
    if !run_pending(store, providers, snapshot, JobKind::Reader, callback).await? {
        return Ok(());
    }
    if !run_pending(store, providers, snapshot, JobKind::Review, callback).await? {
        return Ok(());
    }
    selection::ensure_jobs(snapshot)?;
    if !run_pending(
        store,
        providers,
        snapshot,
        JobKind::SelectEvidence,
        callback,
    )
    .await?
    {
        return Ok(());
    }
    selection::ensure_finals(snapshot)?;
    if !run_pending(store, providers, snapshot, JobKind::Final, callback).await? {
        return Ok(());
    }
    selection::assess(snapshot)?;
    snapshot.checkpoint.stage = RunStage::Ready;
    snapshot.checkpoint.error = None;
    publish(store, snapshot, callback).await
}

async fn run_pending(
    store: &RunStore,
    providers: &[ProviderConnection],
    snapshot: &mut RunSnapshot,
    kind: JobKind,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<bool> {
    snapshot.checkpoint.stage = match kind {
        JobKind::Reader => RunStage::Reading,
        JobKind::Review => RunStage::Reviewing,
        JobKind::SelectEvidence | JobKind::Final => RunStage::FinalReview,
    };
    let models = jobs::run_models(snapshot)?;
    let scheduled: Vec<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.kind == kind && job.state == JobState::Pending)
        .map(|job| ScheduledJob {
            id: job.id.clone(),
            profile_id: job.profile_id.clone(),
        })
        .collect();
    let waves = plan_execution(&scheduled, &models, &snapshot.inputs.execution)?;
    publish(store, snapshot, callback).await?;
    for wave in waves {
        for id in &wave.job_ids {
            if let Some(job) = snapshot
                .checkpoint
                .jobs
                .iter_mut()
                .find(|job| job.id == *id)
            {
                job.state = JobState::Running;
            }
        }
        publish(store, snapshot, callback).await?;
        let mut tasks = tokio::task::JoinSet::new();
        for id in wave.job_ids {
            let job = snapshot
                .checkpoint
                .jobs
                .iter()
                .find(|job| job.id == id)
                .unwrap()
                .clone();
            let model = models
                .iter()
                .find(|model| model.profile.id == job.profile_id)
                .unwrap()
                .clone();
            let matching: Vec<_> = providers
                .iter()
                .filter(|provider| provider.id == model.provider.id)
                .cloned()
                .collect();
            let provider = if matching.len() == 1 {
                matching.into_iter().next()
            } else {
                None
            };
            let store = store.clone();
            let state = snapshot.clone();
            tasks.spawn(async move {
                let result = execute_job(&store, &state, &job, &model, provider).await;
                (id, result)
            });
        }
        while let Some(result) = tasks.join_next().await {
            let (id, result) = result?;
            let job = snapshot
                .checkpoint
                .jobs
                .iter_mut()
                .find(|job| job.id == id)
                .unwrap();
            job.state = match result {
                Ok(response) => {
                    match response.output {
                        JobOutput::Reader(completion) => {
                            snapshot.reader_results.push(super::ReaderResult {
                                job_id: id,
                                completion,
                            })
                        }
                        JobOutput::Review(completion) => {
                            snapshot.review_results.push(super::ReviewResult {
                                job_id: id,
                                completion,
                            })
                        }
                        JobOutput::Final(completion) => {
                            snapshot.final_results.push(super::FinalResult {
                                job_id: id,
                                completion,
                            })
                        }
                        JobOutput::Selection(completion) => {
                            snapshot.selection_results.push(super::SelectionResult {
                                job_id: id,
                                completion,
                            })
                        }
                    }
                    JobState::Complete
                }
                Err(error) => JobState::Failed {
                    message: error.to_string(),
                },
            };
            publish(store, snapshot, callback).await?;
        }
    }
    let failed = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.kind == kind && matches!(job.state, JobState::Failed { .. }))
        .count();
    if failed > 0 {
        snapshot.checkpoint.stage = RunStage::Paused;
        snapshot.checkpoint.error = Some(format!(
            "{failed} model jobs failed. Completed work is saved; Resume retries unfinished jobs."
        ));
        publish(store, snapshot, callback).await?;
        return Ok(false);
    }
    Ok(true)
}

async fn execute_job(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &super::JobRecord,
    model: &crate::model::ModelSnapshot,
    provider: Option<ProviderConnection>,
) -> RunResult<SavedResponse> {
    let provider = provider
        .ok_or_else(|| RunError::Invalid("A selected provider is missing or duplicated".into()))?;
    let preparation_store = store.clone();
    let state = snapshot.clone();
    let preparation_job = job.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        jobs::prepare(&preparation_store, &state, &preparation_job)
    })
    .await??;
    let response = jobs::complete(
        job,
        model,
        &provider,
        Duration::from_secs(snapshot.inputs.execution.request_timeout_seconds),
        prepared,
    )
    .await?;
    let store = store.clone();
    let job = job.clone();
    let run_id = snapshot.inputs.id;
    tokio::task::spawn_blocking(move || {
        store.save_response(run_id, &job, &response)?;
        Ok(response)
    })
    .await?
}

async fn publish(
    store: &RunStore,
    snapshot: &mut RunSnapshot,
    callback: &mut impl FnMut(RunSnapshot),
) -> RunResult<()> {
    let store = store.clone();
    let run_id = snapshot.inputs.id;
    let checkpoint = snapshot.checkpoint.clone();
    tokio::task::spawn_blocking(move || store.save_checkpoint(run_id, &checkpoint)).await??;
    callback(snapshot.clone());
    Ok(())
}
