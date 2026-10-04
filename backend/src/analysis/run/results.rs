//! Structural evidence validation and conservative combination of the two preserved findings.
use super::{RunError, RunResult, model::*};
use std::collections::HashSet;

pub fn validate_response(
    snapshot: &RunSnapshot,
    job: &JobRecord,
    response: &SavedResponse,
) -> RunResult<()> {
    if job.id != response.job_id || job.frame_ids != response.frame_ids {
        return invalid("Response does not belong to its saved request");
    }
    let image_ids: Vec<_> = response
        .sent_images
        .iter()
        .map(|image| &image.candidate_id)
        .collect();
    if (job.kind == JobKind::Consolidate && !image_ids.is_empty())
        || (job.kind != JobKind::Consolidate
            && image_ids != job.frame_ids.iter().collect::<Vec<_>>())
        || response
            .sent_images
            .iter()
            .any(|image| image.size_bytes == 0 || image.sha256.len() != 64)
    {
        return invalid("Saved image identities do not match the request");
    }
    let expected_refs: Vec<_> = snapshot
        .inputs
        .exercise
        .references
        .iter()
        .filter(|reference| {
            snapshot.inputs.evidence_policy.reference_images
                && job.kind != JobKind::Consolidate
                && job
                    .criterion_id
                    .is_none_or(|id| reference.criterion_ids.contains(&id))
        })
        .map(|reference| reference.image.id)
        .collect();
    if response
        .reference_hashes
        .iter()
        .map(|(id, _)| *id)
        .collect::<Vec<_>>()
        != expected_refs
    {
        return invalid("Reference image identities do not match the request");
    }
    match (&job.kind, &response.output) {
        (JobKind::Scan, JobOutput::Scan(completion)) => {
            for observation in &completion.value.observations {
                criterion(snapshot, &observation.criterion_id)?;
                if observation.description.trim().is_empty() || observation.evidence_ids.is_empty()
                {
                    return invalid("An observation needs a visible fact and citations");
                }
                citations(&observation.evidence_ids, &job.frame_ids)?;
            }
        }
        (JobKind::Consolidate, JobOutput::Consolidate(completion)) => {
            let observations = super::jobs::observations(snapshot);
            let mut seen = HashSet::new();
            for finding in &completion.value.findings {
                criterion(snapshot, &finding.criterion_id)?;
                if !seen.insert(&finding.criterion_id) {
                    return invalid("Consolidation repeated a criterion");
                }
                let allowed = observations
                    .iter()
                    .filter(|observation| observation.criterion_id == finding.criterion_id)
                    .flat_map(|observation| observation.evidence_ids.clone())
                    .collect::<Vec<_>>();
                validate_finding(finding, &allowed)?;
            }
            if seen.len() != snapshot.inputs.exercise.checklist.criteria.len() {
                return invalid("Consolidation omitted a criterion");
            }
        }
        (JobKind::Recheck, JobOutput::Recheck(completion)) => {
            if Some(criterion(snapshot, &completion.value.criterion_id)?) != job.criterion_id {
                return invalid("Closer look returned the wrong criterion");
            }
            validate_finding(&completion.value, &job.frame_ids)?;
        }
        _ => return invalid("Response type does not match its request"),
    }
    Ok(())
}

fn criterion(snapshot: &RunSnapshot, id: &str) -> RunResult<uuid::Uuid> {
    snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .find(|item| item.id.to_string() == id)
        .map(|item| item.id)
        .ok_or_else(|| RunError::Invalid("Unknown criterion in model response".into()))
}
fn citations(ids: &[String], allowed: &[String]) -> RunResult<()> {
    let mut seen = HashSet::new();
    if ids
        .iter()
        .any(|id| !allowed.contains(id) || !seen.insert(id))
    {
        return invalid("Citations must name unique candidate frames observed for this criterion");
    }
    Ok(())
}
fn validate_finding(finding: &Finding, allowed: &[String]) -> RunResult<()> {
    citations(&finding.evidence_ids, allowed)?;
    if finding.reason.trim().is_empty() {
        return invalid("A finding needs a reason");
    }
    if finding.judgment != Judgment::CannotVerify
        && (finding.evidence_ids.is_empty()
            || finding.configuration_issue.is_some()
            || finding.conflicting_evidence)
    {
        return invalid(
            "Unsupported, conflicting or policy-dependent findings must remain Cannot verify",
        );
    }
    if finding
        .configuration_issue
        .as_ref()
        .is_some_and(|issue| issue.trim().is_empty())
    {
        return invalid("Configuration issues need an explanation");
    }
    Ok(())
}
fn invalid<T>(message: &str) -> RunResult<T> {
    Err(RunError::Invalid(message.into()))
}

/// Rebuild display results from immutable responses. Contradictory decisions require human review.
pub fn assess(snapshot: &mut RunSnapshot) {
    let initial = snapshot
        .responses
        .iter()
        .find_map(|response| match &response.output {
            JobOutput::Consolidate(completion) => Some(completion.value.findings.clone()),
            _ => None,
        });
    let Some(initial) = initial else {
        return;
    };
    let notes = snapshot
        .checkpoint
        .assessments
        .iter()
        .map(|assessment| (assessment.criterion_id, assessment.recheck_note.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    snapshot.checkpoint.assessments = initial
        .into_iter()
        .map(|initial| {
            let id = uuid::Uuid::parse_str(&initial.criterion_id).expect("validated criterion");
            let recheck = snapshot.responses.iter().find_map(|response| {
                match &response.output {
                    JobOutput::Recheck(completion)
                        if completion.value.criterion_id == initial.criterion_id =>
                    {
                        Some(completion.value.clone())
                    }
                    _ => None,
                }
            });
            let mut outcome = initial.clone();
            if let Some(check) = &recheck {
                // More images can resolve visibility, but cannot define missing exercise policy.
                if initial.configuration_issue.is_none() {
                    if initial.judgment == Judgment::CannotVerify
                        || initial.judgment == check.judgment
                    {
                        outcome = check.clone();
                    } else {
                        outcome.configuration_issue = check.configuration_issue.clone();
                        outcome.judgment = Judgment::CannotVerify;
                        outcome.reason = "The initial assessment and closer look disagree. Review both original findings.".into();
                    }
                }
            }
            let mut evidence_ids = initial.evidence_ids.clone();
            if let Some(check) = &recheck {
                for id in &check.evidence_ids {
                    if !evidence_ids.contains(id) {
                        evidence_ids.push(id.clone());
                    }
                }
            }
            CriterionAssessment {
                criterion_id: id,
                judgment: outcome.judgment,
                reason: outcome.reason,
                evidence_ids,
                configuration_issue: outcome.configuration_issue,
                initial,
                recheck,
                recheck_note: notes.get(&id).cloned().flatten(),
            }
        })
        .collect();
}
