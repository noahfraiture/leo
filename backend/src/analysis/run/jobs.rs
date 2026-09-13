use std::{collections::HashSet, time::Duration};

use serde_json::json;

use super::super::{
    SentImage,
    reconcile::{EvidenceSelection, FinalFinding, ReaderFinding, ReviewFinding},
};
use super::selection_aliases::{ALIASED_PROMPT, AliasedSelection, LEGACY_PROMPT, SelectionAliases};
use super::store::{self, JobOutput, SavedResponse};
use super::{JobKind, JobRecord, RunError, RunResult, RunSnapshot, RunStore};
use crate::model::{JobRole, ModelClient, ModelSnapshot, ProviderConnection, StructuredJob};

const OBSERVATION_RULES: &str = "Evaluate only the supplied operator criterion against candidate images. Reference images teach object identity or state; they never prove candidate behavior. Cite only supplied CANDIDATE image IDs. Apply only the operator checklist rules for retries, ordering, and prohibited actions. Reading instructions or pausing alone is not failure. If an unspecified retry policy changes the judgment, preserve cannotVerify instead of inventing policy. Preserve prohibited actions even if an unrelated later action succeeds. Do not infer hidden internal states. Absence from sampled images is not proof that a step was omitted. Do not join unrelated attempts or assume object identity across ambiguous views. Order needs visible timestamped support. Unknown evidence means cannotVerify, not notMet. Each visible fact needs its own candidate citations. A model's agreement is not evidence.";

/// Prepared bounded bytes and their identities, held only for one scheduled request.
pub struct PreparedJob {
    pub request: StructuredJob,
    pub sent_images: Vec<SentImage>,
    pub reference_hashes: Vec<(uuid::Uuid, String)>,
    pub selection_aliases: Option<SelectionAliases>,
}

/// Creates stable request identity from the immutable run, selected model, criterion, and evidence.
pub fn job_record(
    snapshot: &super::RunInputs,
    criterion_id: uuid::Uuid,
    model: &ModelSnapshot,
    kind: JobKind,
    round: u8,
    frame_ids: Vec<String>,
) -> RunResult<JobRecord> {
    let id = store::hash(&serde_json::to_vec(&(
        snapshot.id,
        &snapshot.prompt_version,
        criterion_id,
        model,
        kind,
        round,
        &frame_ids,
    ))?);
    Ok(JobRecord {
        id,
        criterion_id,
        profile_id: model.profile.id.clone(),
        kind,
        round,
        frame_ids,
        context_job_ids: Vec::new(),
        state: super::JobState::Pending,
    })
}

pub fn run_models(snapshot: &RunSnapshot) -> RunResult<Vec<ModelSnapshot>> {
    let mut models = Vec::new();
    for model in snapshot
        .inputs
        .model_set
        .readers
        .iter()
        .chain([&snapshot.inputs.model_set.reviewer])
    {
        if let Some(existing) = models
            .iter()
            .find(|existing: &&ModelSnapshot| existing.profile.id == model.profile.id)
        {
            if existing != model {
                return Err(RunError::Invalid(
                    "Conflicting snapshots share a model profile ID".into(),
                ));
            }
        } else {
            models.push(model.clone());
        }
    }
    Ok(models)
}

/// Materializes a bounded request off the UI thread. Reused images are verified against their cache hash.
pub fn prepare(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<PreparedJob> {
    if !matches!(
        snapshot.inputs.prompt_version.as_str(),
        LEGACY_PROMPT | ALIASED_PROMPT
    ) {
        return Err(RunError::Invalid("Unknown analysis prompt version".into()));
    }
    if job.kind == JobKind::SelectEvidence && snapshot.inputs.prompt_version == ALIASED_PROMPT {
        let (request, aliases) = super::selector_v2::prepare(snapshot, job, OBSERVATION_RULES)?;
        return Ok(PreparedJob {
            request,
            sent_images: Vec::new(),
            reference_hashes: Vec::new(),
            selection_aliases: Some(aliases),
        });
    }
    let criterion = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .find(|criterion| criterion.id == job.criterion_id)
        .ok_or_else(|| RunError::Invalid("Unknown job criterion".into()))?;
    let super::media::PreparedImages {
        images,
        sent_images,
        reference_hashes,
    } = if job.kind == JobKind::SelectEvidence {
        super::media::PreparedImages {
            images: Vec::new(),
            sent_images: Vec::new(),
            reference_hashes: Vec::new(),
        }
    } else {
        super::media::prepare_images(store, snapshot, job)?
    };
    let relevant_reader_jobs: HashSet<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|reader| {
            reader.criterion_id == job.criterion_id
                && reader.kind == JobKind::Reader
                && reader.frame_ids.iter().any(|id| job.frame_ids.contains(id))
        })
        .map(|reader| &reader.id)
        .collect();
    let reader_findings: Vec<_> = snapshot
        .reader_results
        .iter()
        .filter(|result| relevant_reader_jobs.contains(&result.job_id))
        .collect();
    let review_job_ids: HashSet<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|review| review.criterion_id == job.criterion_id && review.kind == JobKind::Review)
        .map(|review| &review.id)
        .collect();
    let reviews: Vec<_> = snapshot
        .review_results
        .iter()
        .filter(|review| review_job_ids.contains(&review.job_id))
        .collect();
    let review_windows: Vec<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|review| review_job_ids.contains(&review.id))
        .map(|review| json!({"reviewId":review.id,"candidateImageIds":review.frame_ids}))
        .collect();
    let reference_count = snapshot
        .inputs
        .exercise
        .references
        .iter()
        .filter(|reference| reference.criterion_ids.contains(&job.criterion_id))
        .count();
    let image_capacity = snapshot
        .inputs
        .model_set
        .reviewer
        .profile
        .image_limit
        .saturating_sub(reference_count);
    let selection_job_ids: HashSet<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|candidate| {
            candidate.criterion_id == job.criterion_id && candidate.kind == JobKind::SelectEvidence
        })
        .map(|candidate| &candidate.id)
        .collect();
    let selection = snapshot
        .selection_results
        .iter()
        .find(|result| selection_job_ids.contains(&result.job_id));
    let (role, instructions, detail) = match job.kind {
        JobKind::Reader => (
            JobRole::VisualReader,
            format!(
                "{OBSERVATION_RULES} Read independently, without other reader results. Judge only what this bounded window establishes."
            ),
            json!({"reader":job.profile_id}),
        ),
        JobKind::Review => (
            JobRole::VisualReviewer,
            format!(
                "{OBSERVATION_RULES} Visually inspect every supplied frame. Check the individual reader claims against the images even if readers agree. Preserve contradictory observations. If unresolved, request useful camera/time intervals of at most 60 seconds within allowed periods. There is at most one additional evidence round."
            ),
            json!({"readerFindings":reader_findings,"additionalEvidenceRound":job.round,"eligiblePeriods":snapshot.checkpoint.evidence.eligible_periods}),
        ),
        JobKind::SelectEvidence => (
            JobRole::VisualReviewer,
            format!(
                "{OBSERVATION_RULES} This is TEXT-ONLY EVIDENCE SELECTION, not a visual judgment. Account for every supplied visual review ID exactly once as supporting, contradictory, irrelevant, or unresolved. Group IDs with the same explanation to keep output compact. Read every report and the reader claims, including negative claims the reviewer overruled and contradictions inside cannotVerify reports. Choose a small decisive visual comparison within imageCapacity, including support, all material contradictions, and necessary temporal/identity context. Exclude irrelevant background citations with an explicit reason, not by timestamp position or majority. Every contradictory disposition and every required supporting disposition must cite decisive candidate IDs belonging to its named reviews. If a contradiction has no usable images, mark it unresolved and explain why. Set unresolved plus requiredForDecision only if a necessary comparison cannot be resolved from a bounded selection of the available frames; this blocks judgment. Never hide a potentially decisive contrary observation to fit the image cap."
            ),
            json!({"visualReviews":reviews,"reviewWindows":review_windows,"readerFindings":reader_findings,"imageCapacity":image_capacity}),
        ),
        JobKind::Final => (
            JobRole::VisualReviewer,
            format!(
                "{OBSERVATION_RULES} Reconcile all preserved visual reviews below; never use majority voting. Inspect the actual selected supporting, contradictory, and context images. The text-only selector is fallible; check its choices against all reports. Additional contrary evidence may have been included automatically. Do not convert an unselected report into visual proof. If any decisive comparison of identity, attempts, or order remains unseen or ambiguous, set requiresUnseenComparison=true and cannotVerify. Irrelevant background reports do not erase a directly witnessed criterion."
            ),
            json!({"visualReviews":reviews,"reviewWindows":review_windows,"readerFindings":reader_findings,"evidenceSelection":selection}),
        ),
    };
    let request = StructuredJob {
        role,
        instructions,
        text: serde_json::to_string(&json!({
            "jobKind":job.kind,"criterion":criterion,"operatorChecklist":snapshot.inputs.exercise.checklist,
            "candidateImageIds":job.frame_ids,"recordingDurationMs":snapshot.checkpoint.evidence.duration_ms,
            "coverageWarning":"Images are samples. Missing observations cannot establish omission.","detail":detail
        }))?,
        images,
        max_output_tokens: None,
    };
    Ok(PreparedJob {
        request,
        sent_images,
        reference_hashes,
        selection_aliases: None,
    })
}

/// Sends one prepared request and validates its output before it is eligible for durable publication.
pub async fn complete(
    job: &JobRecord,
    model: &ModelSnapshot,
    provider: &ProviderConnection,
    timeout: Duration,
    prepared: PreparedJob,
) -> RunResult<SavedResponse> {
    if provider.snapshot()? != model.provider {
        return Err(RunError::Invalid(
            "Provider connection changed; restore it or start a new run".into(),
        ));
    }
    let client = ModelClient::new(provider, &model.profile, timeout)?;
    let output = match job.kind {
        JobKind::Reader => {
            JobOutput::Reader(client.complete::<ReaderFinding>(&prepared.request).await?)
        }
        JobKind::Review => {
            JobOutput::Review(client.complete::<ReviewFinding>(&prepared.request).await?)
        }
        JobKind::SelectEvidence => match prepared.selection_aliases {
            Some(aliases) => JobOutput::SelectionV2(AliasedSelection {
                aliases,
                completion: client
                    .complete::<super::selection_wire::SelectionWire>(&prepared.request)
                    .await?,
            }),
            None => JobOutput::Selection(
                client
                    .complete::<EvidenceSelection>(&prepared.request)
                    .await?,
            ),
        },
        JobKind::Final => {
            JobOutput::Final(client.complete::<FinalFinding>(&prepared.request).await?)
        }
    };
    Ok(SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: prepared.sent_images,
        reference_hashes: prepared.reference_hashes,
        output,
    })
}
