use std::collections::HashSet;

use super::{EvidenceSelection, FinalFinding, Judgment, ReaderFinding, ReviewFinding, VisualFact};

fn validate(
    judgment: Judgment,
    reason: &str,
    evidence_ids: &[String],
    facts: &[VisualFact],
    provided_ids: &[String],
) -> Result<(), String> {
    if reason.trim().is_empty() {
        return Err("A model judgment needs an explanation".into());
    }
    let provided: HashSet<_> = provided_ids.iter().collect();
    let valid_ids = |ids: &[String]| {
        ids.iter().all(|id| provided.contains(id))
            && ids.iter().collect::<HashSet<_>>().len() == ids.len()
    };
    if !valid_ids(evidence_ids)
        || facts.iter().any(|fact| {
            fact.description.trim().is_empty()
                || fact.evidence_ids.is_empty()
                || !valid_ids(&fact.evidence_ids)
        })
    {
        return Err(
            "Model citations must name unique candidate images provided to this job".into(),
        );
    }
    if judgment != Judgment::CannotVerify && evidence_ids.is_empty() {
        return Err("Met and Not met judgments require candidate evidence".into());
    }
    Ok(())
}

/// Validates candidate IDs against this reader's actual inputs, excluding reference IDs.
pub fn validate_reader(value: &ReaderFinding, provided_ids: &[String]) -> Result<(), String> {
    validate(
        value.judgment,
        &value.reason,
        &value.evidence_ids,
        &value.facts,
        provided_ids,
    )
}

/// Validates citations and basic interval shape. Recorded eligibility is checked by the gatherer.
pub fn validate_review(value: &ReviewFinding, provided_ids: &[String]) -> Result<(), String> {
    validate(
        value.judgment,
        &value.reason,
        &value.evidence_ids,
        &value.facts,
        provided_ids,
    )?;
    if value
        .requested_intervals
        .iter()
        .any(|request| request.start_ms >= request.end_ms)
    {
        return Err("Additional evidence intervals must have a positive duration".into());
    }
    Ok(())
}

/// Requires final citations to have been visually reviewed and preserves unresolved comparisons.
pub fn validate_final(value: &FinalFinding, reviewed_ids: &[String]) -> Result<(), String> {
    validate(
        value.judgment,
        &value.reason,
        &value.evidence_ids,
        &[],
        reviewed_ids,
    )?;
    if value.requires_unseen_comparison && value.judgment != Judgment::CannotVerify {
        return Err(
            "An unseen cross-window comparison cannot establish a candidate judgment".into(),
        );
    }
    Ok(())
}

/// Checks complete review accounting and permitted candidate IDs; relevance remains a model claim.
pub fn validate_selection(
    value: &EvidenceSelection,
    candidate_ids: &[String],
    review_ids: &[String],
) -> Result<(), String> {
    if value.reason.trim().is_empty() {
        return Err("Evidence selection requires a reason".into());
    }
    let expected: HashSet<_> = review_ids.iter().collect();
    let mut accounted = HashSet::new();
    for disposition in &value.reviews {
        if disposition.review_ids.is_empty()
            || disposition.reason.trim().is_empty()
            || disposition
                .review_ids
                .iter()
                .any(|id| !expected.contains(id) || !accounted.insert(id))
        {
            return Err(
                "Every visual review must have exactly one explicit selection disposition".into(),
            );
        }
        if disposition.required_for_decision
            && disposition.kind == super::ReviewDispositionKind::Irrelevant
        {
            return Err("An irrelevant review cannot also be required for the judgment".into());
        }
        if (disposition.kind == super::ReviewDispositionKind::Contradictory
            || (disposition.required_for_decision
                && disposition.kind == super::ReviewDispositionKind::Supporting))
            && disposition.decisive_ids.is_empty()
        {
            return Err(
                "Contradictory and required supporting reviews need decisive candidate IDs".into(),
            );
        }
    }
    if accounted != expected {
        return Err("Evidence selection omitted visual reviews".into());
    }
    let candidates: HashSet<_> = candidate_ids.iter().collect();
    for group in [
        &value.supporting_ids,
        &value.contradicting_ids,
        &value.context_ids,
    ]
    .into_iter()
    .chain(value.reviews.iter().map(|review| &review.decisive_ids))
    {
        if group.iter().any(|id| !candidates.contains(id))
            || group.iter().collect::<HashSet<_>>().len() != group.len()
        {
            return Err("Evidence selection contains unknown or repeated candidate IDs".into());
        }
    }
    Ok(())
}
