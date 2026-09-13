use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A candidate judgment. Provider failures and unfinished processing use separate run states.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema,
)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase")]
pub enum Judgment {
    Met,
    NotMet,
    CannotVerify,
}

/// One model-reported visible fact with candidate citations. The name is not a truth guarantee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualFact {
    pub description: String,
    pub evidence_ids: Vec<String>,
    pub attempt: Option<String>,
    pub identity: Option<String>,
}

/// Independent reader output for one criterion and bounded temporal window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReaderFinding {
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub facts: Vec<VisualFact>,
}

/// A request for more candidate footage, expressed only as camera and session-relative time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRequest {
    pub camera_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Reviewer assessment made while seeing actual candidate frames and reference examples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewFinding {
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub facts: Vec<VisualFact>,
    pub needs_more_evidence: bool,
    pub requested_intervals: Vec<EvidenceRequest>,
}

/// Consolidation over preserved visual reviews, with the actual cited frames when they fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalFinding {
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub unresolved_facts: Vec<String>,
    /// True when a conclusion depends on comparing identity, order, or attempts across images
    /// that were never observed together. Such a conclusion must remain CannotVerify.
    pub requires_unseen_comparison: bool,
}

/// How an evidence selector accounts for a visual review without claiming to inspect its images.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema,
)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase")]
pub enum ReviewDispositionKind {
    Supporting,
    Contradictory,
    Irrelevant,
    Unresolved,
}

/// Reviews sharing one relevance explanation may be grouped to keep selection output compact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDisposition {
    pub review_ids: Vec<String>,
    pub kind: ReviewDispositionKind,
    pub reason: String,
    pub decisive_ids: Vec<String>,
    /// An unresolved requirement blocks a verdict when the available images cannot resolve it.
    pub required_for_decision: bool,
}

/// Text-only proposal of a bounded visual comparison. Actual judgments require the final image pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSelection {
    pub reason: String,
    /// Every supplied visual review ID must appear exactly once across these groups.
    pub reviews: Vec<ReviewDisposition>,
    pub supporting_ids: Vec<String>,
    pub contradicting_ids: Vec<String>,
    pub context_ids: Vec<String>,
}

/// Saved outcome with provenance into individual reader/reviewer outputs and source footage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CriterionAssessment {
    pub criterion_id: Uuid,
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub unresolved_facts: Vec<String>,
    pub reader_job_ids: Vec<String>,
    pub review_job_ids: Vec<String>,
    pub selection_job_id: String,
    /// None when a required unresolved comparison or image limit prevents a visual final pass.
    pub final_job_id: Option<String>,
    pub additional_evidence_round_used: bool,
    pub coverage_note: String,
}
