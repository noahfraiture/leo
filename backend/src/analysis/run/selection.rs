use std::collections::{BTreeSet, HashSet};

use super::super::reconcile::{CriterionAssessment, Judgment, ReviewDispositionKind};
use super::{JobKind, RunError, RunResult, RunSnapshot, jobs, store};

/// Schedules a text-only selector which must account for every visual review of the criterion.
pub fn ensure_jobs(snapshot: &mut RunSnapshot) -> RunResult<()> {
    for criterion in &snapshot.inputs.exercise.checklist.criteria {
        if snapshot
            .checkpoint
            .jobs
            .iter()
            .any(|job| job.criterion_id == criterion.id && job.kind == JobKind::SelectEvidence)
        {
            continue;
        }
        let reviews: Vec<_> = snapshot
            .checkpoint
            .jobs
            .iter()
            .filter(|job| job.criterion_id == criterion.id && job.kind == JobKind::Review)
            .collect();
        let ids: BTreeSet<_> = reviews
            .iter()
            .flat_map(|job| job.frame_ids.iter().cloned())
            .collect();
        let mut job = jobs::job_record(
            &snapshot.inputs,
            criterion.id,
            &snapshot.inputs.model_set.reviewer,
            JobKind::SelectEvidence,
            1,
            ids.into_iter().collect(),
        )?;
        job.context_job_ids = reviews.iter().map(|review| review.id.clone()).collect();
        job.context_job_ids.sort();
        job.id = store::hash(&serde_json::to_vec(&(&job.id, &job.context_job_ids))?);
        snapshot.checkpoint.jobs.push(job);
    }
    Ok(())
}

struct FinalPack {
    selection_job_id: String,
    frame_ids: Vec<String>,
    blocked_reason: Option<String>,
}

/// Combines the selected comparison with explicit contrary observations. Never cuts a prefix.
fn final_pack(snapshot: &RunSnapshot, criterion_id: uuid::Uuid) -> RunResult<FinalPack> {
    let job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.criterion_id == criterion_id && job.kind == JobKind::SelectEvidence)
        .ok_or_else(|| RunError::Invalid("Missing evidence selection job".into()))?;
    let result = snapshot
        .selection_results
        .iter()
        .find(|result| result.job_id == job.id)
        .ok_or_else(|| RunError::Invalid("Missing evidence selection response".into()))?;
    let selection = &result.completion.value;
    let mut frames: BTreeSet<_> = selection
        .supporting_ids
        .iter()
        .chain(&selection.contradicting_ids)
        .chain(&selection.context_ids)
        .cloned()
        .collect();
    let mut unresolved = Vec::new();
    for disposition in &selection.reviews {
        if disposition.kind == ReviewDispositionKind::Unresolved
            && disposition.required_for_decision
        {
            unresolved.push(disposition.reason.clone());
        }
        if disposition.kind != ReviewDispositionKind::Irrelevant {
            frames.extend(disposition.decisive_ids.iter().cloned());
        }
    }
    let criterion_jobs: HashSet<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|candidate| candidate.criterion_id == criterion_id)
        .map(|candidate| &candidate.id)
        .collect();
    // A selector cannot hide an explicit negative observation, including one a reviewer overruled.
    for review in snapshot.review_results.iter().filter(|review| {
        criterion_jobs.contains(&review.job_id)
            && review.completion.value.judgment == Judgment::NotMet
    }) {
        frames.extend(review.completion.value.evidence_ids.iter().cloned());
        frames.extend(
            review
                .completion
                .value
                .facts
                .iter()
                .flat_map(|fact| fact.evidence_ids.iter().cloned()),
        );
    }
    for reader in snapshot.reader_results.iter().filter(|reader| {
        criterion_jobs.contains(&reader.job_id)
            && reader.completion.value.judgment == Judgment::NotMet
    }) {
        frames.extend(reader.completion.value.evidence_ids.iter().cloned());
        frames.extend(
            reader
                .completion
                .value
                .facts
                .iter()
                .flat_map(|fact| fact.evidence_ids.iter().cloned()),
        );
    }
    let references = snapshot
        .inputs
        .exercise
        .references
        .iter()
        .filter(|reference| reference.criterion_ids.contains(&criterion_id))
        .count();
    let capacity = snapshot
        .inputs
        .model_set
        .reviewer
        .profile
        .image_limit
        .saturating_sub(references);
    let blocked_reason = if !unresolved.is_empty() {
        Some(format!(
            "A required comparison remains unresolved: {}",
            unresolved.join("; ")
        ))
    } else if frames.len() > capacity {
        Some(format!(
            "The decisive and contrary evidence requires {} images but this reviewer supports {capacity} after references. No evidence was discarded to produce a verdict.",
            frames.len()
        ))
    } else if frames.is_empty() {
        Some("No decisive candidate image was identified for this criterion.".into())
    } else {
        None
    };
    let mut frame_ids: Vec<_> = frames.into_iter().collect();
    frame_ids.sort_by_key(|id| {
        snapshot
            .checkpoint
            .evidence
            .frame(id)
            .map(|frame| (frame.session_offset_ms, frame.camera_id))
    });
    Ok(FinalPack {
        selection_job_id: job.id.clone(),
        frame_ids,
        blocked_reason,
    })
}

/// Only a complete bounded comparison starts a final image request; blocked cases remain explicit.
pub fn ensure_finals(snapshot: &mut RunSnapshot) -> RunResult<()> {
    for criterion in &snapshot.inputs.exercise.checklist.criteria {
        if snapshot
            .checkpoint
            .jobs
            .iter()
            .any(|job| job.criterion_id == criterion.id && job.kind == JobKind::Final)
        {
            continue;
        }
        let pack = final_pack(snapshot, criterion.id)?;
        if pack.blocked_reason.is_some() {
            continue;
        }
        let mut job = jobs::job_record(
            &snapshot.inputs,
            criterion.id,
            &snapshot.inputs.model_set.reviewer,
            JobKind::Final,
            1,
            pack.frame_ids,
        )?;
        job.context_job_ids.push(pack.selection_job_id);
        snapshot.checkpoint.jobs.push(job);
    }
    Ok(())
}

/// Projects actual final output or an explicit comparison limit, preserving every raw model result.
pub fn assess(snapshot: &mut RunSnapshot) -> RunResult<()> {
    let mut assessments = Vec::new();
    for criterion in &snapshot.inputs.exercise.checklist.criteria {
        let pack = final_pack(snapshot, criterion.id)?;
        let final_job = snapshot
            .checkpoint
            .jobs
            .iter()
            .find(|job| job.criterion_id == criterion.id && job.kind == JobKind::Final);
        let (judgment, reason, evidence_ids, unresolved_facts) =
            if let Some(reason) = pack.blocked_reason {
                (
                    Judgment::CannotVerify,
                    reason.clone(),
                    pack.frame_ids,
                    vec![reason],
                )
            } else {
                let job = final_job
                    .ok_or_else(|| RunError::Invalid("Missing final visual comparison".into()))?;
                let result = snapshot
                    .final_results
                    .iter()
                    .find(|result| result.job_id == job.id)
                    .ok_or_else(|| RunError::Invalid("Missing final visual response".into()))?;
                let finding = &result.completion.value;
                (
                    finding.judgment,
                    finding.reason.clone(),
                    finding.evidence_ids.clone(),
                    finding.unresolved_facts.clone(),
                )
            };
        let coverage_note = snapshot.checkpoint.coverage_notes.iter()
            .find(|(id, _)| *id == criterion.id)
            .map(|(_, note)| note.clone())
            .unwrap_or_else(|| "Every planned window was read and visually reviewed. Selection retained decisive and contrary evidence for actual final image comparison. Sampling cannot prove omission or hidden state.".into());
        assessments.push(CriterionAssessment {
            criterion_id: criterion.id,
            judgment,
            reason,
            evidence_ids,
            unresolved_facts,
            reader_job_ids: jobs_for(snapshot, criterion.id, JobKind::Reader),
            review_job_ids: jobs_for(snapshot, criterion.id, JobKind::Review),
            selection_job_id: pack.selection_job_id,
            final_job_id: final_job.map(|job| job.id.clone()),
            additional_evidence_round_used: snapshot
                .checkpoint
                .gathered_criteria
                .contains(&criterion.id),
            coverage_note,
        });
    }
    snapshot.checkpoint.assessments = assessments;
    Ok(())
}

fn jobs_for(snapshot: &RunSnapshot, criterion_id: uuid::Uuid, kind: JobKind) -> Vec<String> {
    snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.criterion_id == criterion_id && job.kind == kind)
        .map(|job| job.id.clone())
        .collect()
}
