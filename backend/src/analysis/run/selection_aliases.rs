use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{JobRecord, RunError, RunResult, selection_wire::SelectionWire};
use crate::{analysis::EvidenceSelection, model::Completion};

pub const LEGACY_PROMPT: &str = "reference-independent-visual-review-v1";
pub const ALIASED_PROMPT: &str = "reference-independent-visual-review-v2-typed-ids";

/// Positional alias dictionaries: candidate_ids[0] is F001 and review_ids[0] is V001.
/// The dictionaries are copied from the immutable job, not chosen by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionAliases {
    pub candidate_ids: Vec<String>,
    pub review_ids: Vec<String>,
}

impl SelectionAliases {
    pub fn for_job(job: &JobRecord) -> Self {
        Self {
            candidate_ids: job.frame_ids.clone(),
            review_ids: job.context_job_ids.clone(),
        }
    }

    pub fn candidate(&self, id: &str) -> RunResult<String> {
        encode(&self.candidate_ids, "F", id)
    }

    pub fn review(&self, id: &str) -> RunResult<String> {
        encode(&self.review_ids, "V", id)
    }

    /// Translates only known aliases. Repeated IDs within one set are harmless, but repeated
    /// reviews across dispositions remain present so strict accounting rejects the conflict.
    pub fn translate(
        &self,
        value: &EvidenceSelection,
        job: &JobRecord,
    ) -> RunResult<EvidenceSelection> {
        if *self != Self::for_job(job) {
            return Err(RunError::Invalid(
                "Saved selection aliases do not match their immutable job".into(),
            ));
        }
        let mut canonical = value.clone();
        canonical.supporting_ids = decode(&self.candidate_ids, "F", &value.supporting_ids)?;
        canonical.contradicting_ids = decode(&self.candidate_ids, "F", &value.contradicting_ids)?;
        canonical.context_ids = decode(&self.candidate_ids, "F", &value.context_ids)?;
        for disposition in &mut canonical.reviews {
            disposition.review_ids = decode(&self.review_ids, "V", &disposition.review_ids)?;
            disposition.decisive_ids = decode(&self.candidate_ids, "F", &disposition.decisive_ids)?;
        }
        crate::analysis::reconcile::validate_selection(
            &canonical,
            &job.frame_ids,
            &job.context_job_ids,
        )
        .map_err(RunError::Invalid)?;
        Ok(canonical)
    }
}

/// Original aliased completion and the exact dictionaries supplied with its request.
/// Canonical UI/finalizer results are derived from this record without changing the raw reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AliasedSelection {
    pub aliases: SelectionAliases,
    pub completion: Completion<SelectionWire>,
}

impl AliasedSelection {
    pub fn canonical(&self, job: &JobRecord) -> RunResult<Completion<EvidenceSelection>> {
        Ok(Completion {
            value: self
                .aliases
                .translate(&self.completion.value.alias_value(), job)?,
            usage: self.completion.usage.clone(),
            elapsed_ms: self.completion.elapsed_ms,
        })
    }
}

fn encode(ids: &[String], prefix: &str, id: &str) -> RunResult<String> {
    ids.iter()
        .position(|candidate| candidate == id)
        .map(|index| format!("{prefix}{:03}", index + 1))
        .ok_or_else(|| RunError::Invalid("Selection input contains an unknown source ID".into()))
}

fn decode(ids: &[String], prefix: &str, values: &[String]) -> RunResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let index = value
            .strip_prefix(prefix)
            .and_then(|number| number.parse::<usize>().ok())
            .and_then(|number| number.checked_sub(1))
            .filter(|index| format!("{prefix}{:03}", index + 1) == *value)
            .filter(|index| *index < ids.len())
            .ok_or_else(|| {
                RunError::Invalid("Selection contains an unknown review or candidate alias".into())
            })?;
        if seen.insert(index) {
            result.push(ids[index].clone());
        }
    }
    Ok(result)
}
