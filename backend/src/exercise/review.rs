use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Error, ExerciseDraft, Result};

/// A half-open UTF-8 byte range in the exact submitted source text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}

impl SourceRange {
    fn is_valid(&self, source: &str) -> bool {
        self.start < self.end && source.get(self.start..self.end).is_some()
    }
}

/// Suggested wording linked to the supplied source; it is not an accepted criterion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedCriterion {
    pub text: String,
    pub source_ranges: Vec<SourceRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCommentKind {
    Ambiguity,
    SourceDiscrepancy,
    SuggestedChange,
}

/// Questions and proposed policy changes stay separate from the suggested checklist text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalComment {
    pub kind: ProposalCommentKind,
    pub text: String,
    pub source_ranges: Vec<SourceRange>,
    pub proposed_item: Option<usize>,
}

/// Temporary assistance bound to a complete draft snapshot, including unsaved local edits.
/// Validation checks structure and freshness, not the semantic faithfulness of the rewrite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChecklistProposal {
    pub exercise_id: Uuid,
    pub edit_version: u64,
    pub draft_fingerprint: String,
    pub source_text: String,
    pub criteria: Vec<ProposedCriterion>,
    pub comments: Vec<ProposalComment>,
}

impl ChecklistProposal {
    /// Bind a response to the request's captured draft, without publishing or editing it.
    pub fn for_draft(
        draft: &ExerciseDraft,
        criteria: Vec<ProposedCriterion>,
        comments: Vec<ProposalComment>,
    ) -> Result<Self> {
        let proposal = Self {
            exercise_id: draft.id,
            edit_version: draft.edit_version,
            draft_fingerprint: draft_fingerprint(draft)?,
            source_text: draft.checklist.source_text(),
            criteria,
            comments,
        };
        proposal.validate_against(draft)?;
        Ok(proposal)
    }

    /// Reject responses for changed drafts and invalid source/comment anchors.
    pub fn validate_against(&self, draft: &ExerciseDraft) -> Result<()> {
        if self.exercise_id != draft.id
            || self.edit_version != draft.edit_version
            || self.draft_fingerprint != draft_fingerprint(draft)?
            || self.source_text != draft.checklist.source_text()
        {
            return Err(Error::StaleProposal);
        }
        if self.criteria.is_empty()
            || self.criteria.iter().any(|item| {
                item.text.trim().is_empty()
                    || item.source_ranges.is_empty()
                    || item
                        .source_ranges
                        .iter()
                        .any(|range| !range.is_valid(&self.source_text))
            })
        {
            return Err(Error::Invalid(
                "proposal items need wording and valid source ranges".into(),
            ));
        }
        for comment in &self.comments {
            if comment.text.trim().is_empty()
                || comment
                    .proposed_item
                    .is_some_and(|index| index >= self.criteria.len())
                || comment
                    .source_ranges
                    .iter()
                    .any(|range| !range.is_valid(&self.source_text))
                || (comment.kind == ProposalCommentKind::SourceDiscrepancy
                    && comment.source_ranges.is_empty())
            {
                return Err(Error::Invalid(
                    "proposal comment has an invalid or missing anchor".into(),
                ));
            }
        }
        Ok(())
    }
}

fn draft_fingerprint(draft: &ExerciseDraft) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(draft)?)))
}
