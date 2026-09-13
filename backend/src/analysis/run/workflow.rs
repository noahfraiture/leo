use std::collections::{BTreeMap, HashSet};

use super::super::{EvidenceInterval, gather_evidence_with_budget, plan_windows};
use super::{JobKind, RunError, RunResult, RunSnapshot, jobs};

/// Every initial window is visually reviewed, including windows where readers agree.
pub fn ensure_reviews(snapshot: &mut RunSnapshot) -> RunResult<()> {
    for criterion in &snapshot.inputs.windows {
        for window in &criterion.windows {
            let job = jobs::job_record(
                &snapshot.inputs,
                criterion.criterion_id,
                &snapshot.inputs.model_set.reviewer,
                JobKind::Review,
                0,
                window.frame_ids.clone(),
            )?;
            if !snapshot
                .checkpoint
                .jobs
                .iter()
                .any(|existing| existing.id == job.id)
            {
                snapshot.checkpoint.jobs.push(job);
            }
        }
    }
    Ok(())
}

/// Allows one bounded round per criterion, with a durable record even when no new sample is available.
pub fn gather(snapshot: &mut RunSnapshot) -> RunResult<()> {
    let criteria: Vec<_> = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .map(|criterion| criterion.id)
        .collect();
    for criterion_id in criteria {
        if snapshot
            .checkpoint
            .gathered_criteria
            .contains(&criterion_id)
        {
            continue;
        }
        let reviews = reviews_for(snapshot, criterion_id);
        if !reviews
            .iter()
            .any(|review| review.completion.value.needs_more_evidence)
        {
            continue;
        }
        let mut intervals: Vec<_> = reviews
            .iter()
            .flat_map(|review| &review.completion.value.requested_intervals)
            .map(|request| EvidenceInterval {
                camera_id: request.camera_id,
                start_ms: request.start_ms,
                end_ms: request.end_ms,
            })
            .collect();
        if intervals.is_empty() {
            // With no located action, spread a bounded search across eligible periods rather than
            // repeatedly inspecting the earliest uncertain window.
            for period in &snapshot.checkpoint.evidence.eligible_periods {
                for part in 0..3 {
                    let start_ms = period.start_ms
                        + (period.end_ms - period.start_ms) * part / 3
                        + period.sample_every_ms / 2;
                    if start_ms < period.end_ms {
                        intervals.push(EvidenceInterval {
                            camera_id: period.camera_id,
                            start_ms,
                            end_ms: (start_ms + 2000).min(period.end_ms),
                        });
                    }
                }
            }
        }
        let requested_count = intervals.len();
        if intervals.len() > 8 {
            intervals = (0..8)
                .map(|index| intervals[index * requested_count / 8].clone())
                .collect();
        }
        let mut added = Vec::new();
        let mut rejected = 0;
        let interval_count = intervals.len();
        // Search independently per criterion. Globally cached samples still need this
        // criterion's own reader and reviewer jobs.
        let mut selection = snapshot.inputs.initial_evidence.clone();
        for (index, interval) in intervals.into_iter().enumerate() {
            let budget = (128 - added.len()) / (interval_count - index);
            match gather_evidence_with_budget(&mut selection, &[interval], budget) {
                Ok(ids) => added.extend(ids),
                Err(_) => rejected += 1,
            }
        }
        for id in &added {
            let frame = selection
                .frame(id)
                .ok_or_else(|| RunError::Invalid("Missing selected revisit frame".into()))?;
            match snapshot.checkpoint.evidence.frame(id) {
                Some(existing) if existing != frame => {
                    return Err(RunError::Invalid("Revisit frame identity changed".into()));
                }
                Some(_) => {}
                None => snapshot.checkpoint.evidence.frames.push(frame.clone()),
            }
        }
        snapshot
            .checkpoint
            .evidence
            .frames
            .sort_by_key(|frame| (frame.session_offset_ms, frame.camera_id));
        snapshot.checkpoint.gathered_criteria.push(criterion_id);
        snapshot.checkpoint.coverage_notes.push((criterion_id, format!(
            "One additional round selected {} denser candidate images for this criterion from {} requested/search intervals (at most 8 intervals and 128 images; {} unavailable intervals). This is partial sampling, not proof of omission.",
            added.len(), requested_count, rejected
        )));
        if added.is_empty() {
            continue;
        }
        let mut additional = snapshot.checkpoint.evidence.clone();
        let mut sets = BTreeMap::<u64, Vec<String>>::new();
        for id in added {
            let frame = additional
                .frame(&id)
                .ok_or_else(|| RunError::Invalid("Missing added frame".into()))?;
            sets.entry(frame.session_offset_ms).or_default().push(id);
        }
        additional.frame_sets = sets.into_values().collect();
        let references = snapshot
            .inputs
            .exercise
            .references
            .iter()
            .filter(|reference| reference.criterion_ids.contains(&criterion_id))
            .count();
        let limit = snapshot
            .inputs
            .model_set
            .readers
            .iter()
            .chain([&snapshot.inputs.model_set.reviewer])
            .map(|model| model.profile.image_limit)
            .min()
            .unwrap();
        let windows = plan_windows(
            &additional,
            limit,
            references,
            snapshot.inputs.evidence_policy.max_window_span_ms,
            0,
        )?;
        for window in windows {
            for model in &snapshot.inputs.model_set.readers {
                snapshot.checkpoint.jobs.push(jobs::job_record(
                    &snapshot.inputs,
                    criterion_id,
                    model,
                    JobKind::Reader,
                    1,
                    window.frame_ids.clone(),
                )?);
            }
            snapshot.checkpoint.jobs.push(jobs::job_record(
                &snapshot.inputs,
                criterion_id,
                &snapshot.inputs.model_set.reviewer,
                JobKind::Review,
                1,
                window.frame_ids,
            )?);
        }
    }
    Ok(())
}

fn reviews_for(
    snapshot: &RunSnapshot,
    criterion_id: uuid::Uuid,
) -> Vec<&super::model::ReviewResult> {
    let jobs: HashSet<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.criterion_id == criterion_id && job.kind == JobKind::Review)
        .map(|job| &job.id)
        .collect();
    snapshot
        .review_results
        .iter()
        .filter(|result| jobs.contains(&result.job_id))
        .collect()
}
