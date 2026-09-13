use serde_json::{Value, json};

use super::{
    JobKind, JobRecord, RunError, RunResult, RunSnapshot, selection_aliases::SelectionAliases,
};
use crate::model::{JobRole, StructuredJob};

/// Builds the v2 text-only selector prompt without usage metadata, hash IDs, or duplicate
/// review/window documents. Every original finding remains present with typed citations renamed.
pub fn prepare(
    snapshot: &RunSnapshot,
    job: &JobRecord,
    rules: &str,
) -> RunResult<(StructuredJob, SelectionAliases)> {
    let aliases = SelectionAliases::for_job(job);
    let criterion = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .find(|criterion| criterion.id == job.criterion_id)
        .ok_or_else(|| RunError::Invalid("Unknown selection criterion".into()))?;
    let mut candidates = Vec::new();
    for id in &job.frame_ids {
        let frame = snapshot
            .checkpoint
            .evidence
            .frame(id)
            .ok_or_else(|| RunError::Invalid("Unknown selection candidate".into()))?;
        candidates.push(json!({"id":aliases.candidate(id)?,"camera":frame.camera_id,"timeMs":frame.session_offset_ms}));
    }
    let mut reviews = Vec::new();
    for id in &job.context_job_ids {
        let review_job = snapshot
            .checkpoint
            .jobs
            .iter()
            .find(|source| {
                source.id == *id
                    && source.kind == JobKind::Review
                    && source.criterion_id == job.criterion_id
            })
            .ok_or_else(|| RunError::Invalid("Unknown selection source review".into()))?;
        let review = snapshot
            .review_results
            .iter()
            .find(|review| review.job_id == *id)
            .ok_or_else(|| RunError::Invalid("Missing selection source review".into()))?;
        reviews.push(json!({
            "jobId":aliases.review(id)?,
            "frameIds":review_job.frame_ids.iter().map(|id| aliases.candidate(id)).collect::<RunResult<Vec<_>>>()?,
            "completion":{"value":finding(&review.completion.value, &aliases)?}
        }));
    }
    let mut readers = Vec::new();
    for source in snapshot.checkpoint.jobs.iter().filter(|source| {
        source.kind == JobKind::Reader
            && source.criterion_id == job.criterion_id
            && source.frame_ids.iter().any(|id| job.frame_ids.contains(id))
    }) {
        let result = snapshot
            .reader_results
            .iter()
            .find(|result| result.job_id == source.id)
            .ok_or_else(|| RunError::Invalid("Missing selection reader finding".into()))?;
        readers.push(json!({
            "reader":source.profile_id,
            "frameIds":source.frame_ids.iter().map(|id| aliases.candidate(id)).collect::<RunResult<Vec<_>>>()?,
            "finding":finding(&result.completion.value, &aliases)?
        }));
    }
    let references = snapshot
        .inputs
        .exercise
        .references
        .iter()
        .filter(|reference| reference.criterion_ids.contains(&job.criterion_id))
        .count();
    let request = StructuredJob {
        role: JobRole::VisualReviewer,
        instructions: format!(
            "{rules} This job only selects evidence from TEXT reports, not images. Use only V-prefixed review IDs and F-prefixed candidate IDs exactly as supplied. Only visualReviewIds may contain V IDs. Every field ending CandidateFrameIds must contain F IDs, never V IDs. Account for every V ID exactly once across the review disposition groups. Group reviews with the same explanation. Do not copy IDs from narrative text. Select the smallest sufficient comparison of supporting, contrary, and temporal/identity context within imageCapacity. Read every report and reader finding, including negatives overruled by reviewers and contradictions inside cannotVerify. Every contradictory group and required supporting group needs decisive F IDs belonging to its named review windows. If a necessary comparison has no usable evidence, mark it unresolved and requiredForDecision. Do not omit reviews or hide contradictions to fit the output or image limit. Return each ID only once within a list. Irrelevant background needs an explicit reason; never select by majority or timestamp prefix."
        ),
        text: serde_json::to_string(&json!({
            "jobKind":"selectEvidence","contract":"aliases-v2-typed-ids", "criterion":criterion.text,
            "operatorChecklist":snapshot.inputs.exercise.checklist.criteria.iter().map(|criterion| &criterion.text).collect::<Vec<_>>(),
            "recordingDurationMs":snapshot.checkpoint.evidence.duration_ms,
            "candidateFrames":candidates,
            "coverageWarning":"Images are samples. Missing observations cannot establish omission.",
            "detail":{"visualReviews":reviews,"readerFindings":readers,
                "imageCapacity":snapshot.inputs.model_set.reviewer.profile.image_limit.saturating_sub(references)}
        }))?,
        images: Vec::new(),
        max_output_tokens: None,
    };
    Ok((request, aliases))
}

fn finding(value: &impl serde::Serialize, aliases: &SelectionAliases) -> RunResult<Value> {
    let mut value = serde_json::to_value(value)?;
    for id in value["evidenceIds"].as_array_mut().into_iter().flatten() {
        *id = json!(
            aliases.candidate(
                id.as_str()
                    .ok_or_else(|| RunError::Invalid("Invalid finding citation".into()))?
            )?
        );
    }
    for fact in value["facts"].as_array_mut().into_iter().flatten() {
        for id in fact["evidenceIds"].as_array_mut().into_iter().flatten() {
            *id = json!(
                aliases.candidate(
                    id.as_str()
                        .ok_or_else(|| RunError::Invalid("Invalid fact citation".into()))?
                )?
            );
        }
    }
    Ok(value)
}
