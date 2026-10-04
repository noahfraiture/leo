//! Stateless requests. The closer look receives no earlier verdict or model explanation.
use super::{RunError, RunResult, RunStore, model::*, store};
use crate::model::{JobRole, ModelClient, ProviderConnection, StructuredJob};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

pub const PROMPT_VERSION: &str = "leo-whole-checklist-v1";
const RULES: &str = "Apply only the confirmed operator checklist, scene context, order policy and operator answers. Questions without answers are not requirements. If an unspecified policy affects the decision, flag configurationIssue and cannotVerify rather than inventing a standard. Reference images explain objects and states; they never prove student behavior. Cite only candidate image IDs. Images sample the recording: an action not seen is not proof it was omitted. Do not infer hidden states or merge unrelated attempts. Preserve visible contradictions, prohibited actions and retries; do not count matching reports as votes. Pausing or reading instructions alone is not a failure.";

pub fn record(
    inputs: &RunInputs,
    kind: JobKind,
    criterion_id: Option<Uuid>,
    frame_ids: Vec<String>,
) -> RunResult<JobRecord> {
    let id = store::hash(&serde_json::to_vec(&(
        inputs.id,
        kind,
        criterion_id,
        &frame_ids,
    ))?);
    Ok(JobRecord {
        id,
        kind,
        criterion_id,
        frame_ids,
        state: JobState::Pending,
        failures: Vec::new(),
    })
}

pub struct PreparedJob {
    pub request: StructuredJob,
    pub sent_images: Vec<crate::analysis::SentImage>,
    pub reference_hashes: Vec<(Uuid, String)>,
}

pub fn prepare(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<PreparedJob> {
    if snapshot.inputs.prompt_version != PROMPT_VERSION {
        return Err(RunError::Invalid(
            "This run uses an unsupported analysis prompt".into(),
        ));
    }
    let (instructions, detail) = match job.kind {
        JobKind::Scan => (
            "Observe this chronological window against the entire checklist. Return sparse factual observations linked to criterionId and cited frames, including contrary evidence and attempts. Describe what visibly happens, not a grade. Leave unobserved criteria out. A static end state does not prove the preceding action or order.",
            json!({"checklist":snapshot.inputs.exercise.checklist}),
        ),
        JobKind::Consolidate => (
            "Summarize ALL saved observations into exactly one finding per checklist criterion. This is text consolidation, not a new visual inspection. Do not create visual facts. Cite only that criterion's observation citations. Check ordering across timestamps using the operator order policy. A single matching window does not erase contrary evidence or establish full-recording prohibitions. Set conflictingEvidence=true and cannotVerify when material observations conflict. A missing observation or insufficient temporal coverage means cannotVerify. Preserve useful candidate citations even for uncertain findings, so a bounded independent closer look can examine them.",
            json!({"checklist":snapshot.inputs.exercise.checklist,"observations":observations(snapshot)}),
        ),
        JobKind::Recheck => {
            let criterion = snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .iter()
                .find(|item| Some(item.id) == job.criterion_id)
                .ok_or_else(|| RunError::Invalid("Unknown closer-look criterion".into()))?;
            (
                "Independently assess this criterion using only the supplied original candidate images and operator configuration. You have one bounded episode, not the whole recording. Do not claim globally required order, absence or repetition without sufficient visible evidence. If a missing part of the recording is necessary, return cannotVerify. Ignore what an earlier evaluator might have concluded.",
                json!({"criterion":criterion,"checklist":snapshot.inputs.exercise.checklist}),
            )
        }
    };
    let media = if job.kind == JobKind::Consolidate {
        super::media::PreparedImages {
            images: Vec::new(),
            sent_images: Vec::new(),
            reference_hashes: Vec::new(),
        }
    } else {
        super::media::prepare_images(store, snapshot, job)?
    };
    let frames: Vec<_> = job
        .frame_ids
        .iter()
        .filter_map(|id| snapshot.checkpoint.evidence.frame(id))
        .collect();
    let preparation = &snapshot.inputs.exercise.preparation;
    let text = serde_json::to_string(&json!({
        "jobKind": job.kind, "detail": detail,
        "sceneContext":preparation.scene_context,"orderPolicy":preparation.order,
        "operatorClarifications":preparation.questions.iter().filter(|question| snapshot.inputs.exercise.checklist.criteria.iter().any(|criterion| criterion.id == question.criterion_id)).collect::<Vec<_>>(),
        "referenceLabels":snapshot.inputs.exercise.references,
        "candidateFrames":frames,"recordingDurationMs":snapshot.checkpoint.evidence.duration_ms,
        "recordingGaps":snapshot.checkpoint.evidence.warnings,
    }))?;
    Ok(PreparedJob {
        request: StructuredJob {
            role: if job.kind == JobKind::Scan {
                JobRole::VisualReader
            } else {
                JobRole::VisualReviewer
            },
            instructions: format!("{RULES}\n{instructions}"),
            text,
            images: media.images,
            max_output_tokens: None,
        },
        sent_images: media.sent_images,
        reference_hashes: media.reference_hashes,
    })
}

pub fn observations(snapshot: &RunSnapshot) -> Vec<&ScanObservation> {
    snapshot
        .responses
        .iter()
        .filter_map(|response| match &response.output {
            JobOutput::Scan(completion) => Some(&completion.value.observations),
            _ => None,
        })
        .flatten()
        .collect()
}

pub async fn complete(
    snapshot: &RunSnapshot,
    job: &JobRecord,
    provider: &ProviderConnection,
    prepared: PreparedJob,
) -> RunResult<SavedResponse> {
    let model = &snapshot.inputs.model;
    if provider.snapshot()? != model.provider {
        return Err(RunError::Invalid(
            "The saved model connection changed; start a new run to use it".into(),
        ));
    }
    let client = ModelClient::new(
        provider,
        &model.profile,
        Duration::from_secs(snapshot.inputs.execution.request_timeout_seconds),
    )?;
    let output = match job.kind {
        JobKind::Scan => JobOutput::Scan(client.complete(&prepared.request).await?),
        JobKind::Consolidate => JobOutput::Consolidate(client.complete(&prepared.request).await?),
        JobKind::Recheck => JobOutput::Recheck(client.complete(&prepared.request).await?),
    };
    let response = SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: prepared.sent_images,
        reference_hashes: prepared.reference_hashes,
        output,
    };
    super::results::validate_response(snapshot, job, &response)?;
    Ok(response)
}
