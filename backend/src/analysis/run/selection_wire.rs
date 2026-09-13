use serde::{Deserialize, Serialize};

use crate::analysis::{EvidenceSelection, ReviewDisposition, ReviewDispositionKind};

/// Model-facing v2 selection. Explicit namespaces and schema patterns prevent review IDs from
/// being proposed as candidate frames; membership and source provenance still require validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionWire {
    pub reason: String,
    pub reviews: Vec<ReviewDispositionWire>,
    #[schemars(inner(regex(pattern = "^F[0-9]{3,}$")))]
    pub supporting_candidate_frame_ids: Vec<String>,
    #[schemars(inner(regex(pattern = "^F[0-9]{3,}$")))]
    pub contradicting_candidate_frame_ids: Vec<String>,
    #[schemars(inner(regex(pattern = "^F[0-9]{3,}$")))]
    pub context_candidate_frame_ids: Vec<String>,
}

/// A relevance explanation for one or more visual reports, citing only their candidate frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDispositionWire {
    #[schemars(inner(regex(pattern = "^V[0-9]{3,}$")))]
    pub visual_review_ids: Vec<String>,
    pub kind: ReviewDispositionKind,
    pub reason: String,
    #[schemars(inner(regex(pattern = "^F[0-9]{3,}$")))]
    pub decisive_candidate_frame_ids: Vec<String>,
    pub required_for_decision: bool,
}

impl SelectionWire {
    /// Copies the wire fields for strict alias translation, keeping the original reply intact.
    pub fn alias_value(&self) -> EvidenceSelection {
        EvidenceSelection {
            reason: self.reason.clone(),
            reviews: self
                .reviews
                .iter()
                .map(|review| ReviewDisposition {
                    review_ids: review.visual_review_ids.clone(),
                    kind: review.kind,
                    reason: review.reason.clone(),
                    decisive_ids: review.decisive_candidate_frame_ids.clone(),
                    required_for_decision: review.required_for_decision,
                })
                .collect(),
            supporting_ids: self.supporting_candidate_frame_ids.clone(),
            contradicting_ids: self.contradicting_candidate_frame_ids.clone(),
            context_ids: self.context_candidate_frame_ids.clone(),
        }
    }
}
