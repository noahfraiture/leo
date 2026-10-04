//! One bounded episode, chosen from existing citations or an explicit operator-selected timestamp.
use super::{RunError, RunResult, jobs, model::*};
use crate::analysis::{EvidenceInterval, gather_evidence_with_budget};
use uuid::Uuid;

/// Queue at most one closer look. Never discard distant or over-budget citations to force a verdict.
pub fn queue(
    snapshot: &mut RunSnapshot,
    criterion_id: Uuid,
    selected_frame: Option<&str>,
) -> RunResult<()> {
    if snapshot
        .checkpoint
        .jobs
        .iter()
        .any(|job| job.kind == JobKind::Recheck && job.criterion_id == Some(criterion_id))
    {
        return Err(RunError::Invalid(
            "This criterion already has its one closer look. Resume it if processing failed."
                .into(),
        ));
    }
    let assessment = snapshot
        .checkpoint
        .assessments
        .iter()
        .find(|assessment| assessment.criterion_id == criterion_id)
        .ok_or_else(|| {
            RunError::Invalid("Wait for the initial assessment before a closer look".into())
        })?;
    if assessment.configuration_issue.is_some() {
        return Err(RunError::Invalid("Clarify the exercise policy before starting a new analysis; more images cannot decide operator policy".into()));
    }
    let mut required = assessment.initial.evidence_ids.clone();
    if let Some(id) = selected_frame
        && !required.iter().any(|item| item == id)
    {
        required.push(id.into());
    }
    let frames = required
        .iter()
        .map(|id| {
            snapshot
                .checkpoint
                .evidence
                .frame(id)
                .cloned()
                .ok_or_else(|| RunError::Invalid("Unknown candidate frame".into()))
        })
        .collect::<RunResult<Vec<_>>>()?;
    let Some(start) = frames.iter().map(|frame| frame.session_offset_ms).min() else {
        return Err(RunError::Invalid(
            "No episode was located. Select a candidate timestamp for the closer look.".into(),
        ));
    };
    let end = frames
        .iter()
        .map(|frame| frame.session_offset_ms)
        .max()
        .unwrap();
    let start = start.saturating_sub(2_000);
    let end = end
        .saturating_add(2_000)
        .min(snapshot.checkpoint.evidence.duration_ms);
    if end <= start || end - start > 60_000 {
        return Err(RunError::Invalid(
            "The evidence spans more than one bounded episode. Review it manually.".into(),
        ));
    }
    let references = if snapshot.inputs.evidence_policy.reference_images {
        snapshot
            .inputs
            .exercise
            .references
            .iter()
            .filter(|reference| reference.criterion_ids.contains(&criterion_id))
            .count()
    } else {
        0
    };
    let capacity = snapshot
        .inputs
        .model
        .profile
        .image_limit
        .saturating_sub(references);
    if capacity <= required.len() {
        return Err(RunError::Invalid("No room for additional images without dropping original evidence. Review manually or use a new run with a larger image limit.".into()));
    }
    let mut cameras = frames
        .iter()
        .map(|frame| frame.camera_id)
        .collect::<Vec<_>>();
    cameras.sort_unstable();
    cameras.dedup();
    let intervals = cameras
        .into_iter()
        .map(|camera_id| EvidenceInterval {
            camera_id,
            start_ms: start,
            end_ms: end,
        })
        .collect::<Vec<_>>();
    // Reuse the same independently planned episode even if another criterion gathered it first.
    let mut evidence = snapshot.inputs.initial_evidence.clone();
    for frame in &frames {
        if evidence.frame(&frame.id).is_none() {
            evidence.frames.push(frame.clone());
        }
    }
    let extra = gather_evidence_with_budget(
        &mut evidence,
        &intervals,
        (capacity - required.len()).min(128),
    )?;
    if extra.is_empty() {
        return Err(RunError::Invalid(
            "No additional recorded images are available in this episode".into(),
        ));
    }
    required.extend(extra);
    required.sort_by_key(|id| {
        evidence
            .frame(id)
            .map(|frame| (frame.session_offset_ms, frame.camera_id))
    });
    let job = jobs::record(
        &snapshot.inputs,
        JobKind::Recheck,
        Some(criterion_id),
        required,
    )?;
    for frame in evidence.frames {
        if snapshot.checkpoint.evidence.frame(&frame.id).is_none() {
            snapshot.checkpoint.evidence.frames.push(frame);
        }
    }
    snapshot
        .checkpoint
        .evidence
        .frames
        .sort_by_key(|frame| (frame.session_offset_ms, frame.camera_id));
    snapshot.checkpoint.jobs.push(job);
    snapshot.checkpoint.stage = RunStage::Rechecking;
    Ok(())
}

pub fn queue_automatic(snapshot: &mut RunSnapshot) {
    if !snapshot.inputs.evidence_policy.automatic_recheck {
        return;
    }
    let candidates: Vec<_> = snapshot
        .checkpoint
        .assessments
        .iter()
        .filter(|assessment| {
            assessment.initial.judgment == Judgment::CannotVerify
                && assessment.configuration_issue.is_none()
                && assessment.recheck_note.is_none()
                && !snapshot.checkpoint.jobs.iter().any(|job| {
                    job.kind == JobKind::Recheck
                        && job.criterion_id == Some(assessment.criterion_id)
                })
        })
        .map(|assessment| assessment.criterion_id)
        .collect();
    for id in candidates {
        if let Err(error) = queue(snapshot, id, None)
            && let Some(assessment) = snapshot
                .checkpoint
                .assessments
                .iter_mut()
                .find(|assessment| assessment.criterion_id == id)
        {
            assessment.recheck_note = Some(error.to_string());
        }
    }
}
