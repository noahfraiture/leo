use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::reconcile::{
    CriterionAssessment, EvidenceSelection, FinalFinding, Judgment, ReaderFinding, ReviewFinding,
};
use super::super::{EvidenceManifest, EvidenceWindow};
use crate::{
    exercise::ExerciseRevision,
    model::{Completion, ExecutionPolicy, ResolvedModelSet},
    profiles::{ImageDetailPolicy, ImageSizePolicy},
};

/// Shared evidence preparation settings, independent of the chosen model providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidencePolicy {
    pub max_window_span_ms: u64,
    pub overlap_frame_sets: usize,
    pub image_size: ImageSizePolicy,
    pub image_detail: ImageDetailPolicy,
}

impl Default for EvidencePolicy {
    fn default() -> Self {
        Self {
            max_window_span_ms: 7000,
            overlap_frame_sets: 1,
            image_size: ImageSizePolicy::MaximumLongEdge(1280),
            image_detail: ImageDetailPolicy::ProviderDefault,
        }
    }
}

/// Complete immutable configuration and initial recording evidence for reproducibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunInputs {
    pub schema_version: u8,
    pub id: Uuid,
    pub created_utc_ms: u64,
    pub prompt_version: String,
    pub exercise: ExerciseRevision,
    pub model_set: ResolvedModelSet,
    pub execution: ExecutionPolicy,
    pub evidence_policy: EvidencePolicy,
    pub initial_evidence: EvidenceManifest,
    /// Windows are criterion-specific because their required reference counts can differ.
    pub windows: Vec<CriterionWindows>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CriterionWindows {
    pub criterion_id: Uuid,
    pub windows: Vec<EvidenceWindow>,
}

/// Durable execution phase, independent of candidate judgments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStage {
    Preparing,
    Reading,
    Reviewing,
    Gathering,
    FinalReview,
    Ready,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Reader,
    Review,
    SelectEvidence,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum JobState {
    Pending,
    Running,
    Complete,
    Failed { message: String },
}

/// Stable request identity and state. Only candidate image IDs can be cited by its output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobRecord {
    pub id: String,
    pub criterion_id: Uuid,
    pub profile_id: String,
    pub kind: JobKind,
    pub round: u8,
    /// Actual candidate images for visual jobs. For text-only selection, these are the available
    /// image IDs described in reviews; no image bytes are sent to that selector.
    pub frame_ids: Vec<String>,
    /// Durable model outputs supplied as context; selection must account for every listed review.
    pub context_job_ids: Vec<String>,
    pub state: JobState,
}

/// Saved progress; accepted response files remain authoritative if a checkpoint write was interrupted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunCheckpoint {
    pub stage: RunStage,
    pub jobs: Vec<JobRecord>,
    pub evidence: EvidenceManifest,
    pub gathered_criteria: Vec<Uuid>,
    pub coverage_notes: Vec<(Uuid, String)>,
    pub assessments: Vec<CriterionAssessment>,
    pub error: Option<String>,
}

/// Persisted typed responses preserve independent model findings and usage separately from outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReaderResult {
    pub job_id: String,
    pub completion: Completion<ReaderFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewResult {
    pub job_id: String,
    pub completion: Completion<ReviewFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalResult {
    pub job_id: String,
    pub completion: Completion<FinalFinding>,
}

/// A preserved text-only selection proposal with explicit dispositions of every visual review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionResult {
    pub job_id: String,
    pub completion: Completion<EvidenceSelection>,
}

/// Operator correction appended independently of model outputs and exercise configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanCorrection {
    pub id: Uuid,
    pub criterion_id: Uuid,
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub uses_outside_information: bool,
    pub created_utc_ms: u64,
    pub supersedes: Option<Uuid>,
}

/// UI projection with frozen inputs, current progress, original outputs, and separate corrections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSnapshot {
    pub inputs: RunInputs,
    pub checkpoint: RunCheckpoint,
    pub reader_results: Vec<ReaderResult>,
    pub review_results: Vec<ReviewResult>,
    pub final_results: Vec<FinalResult>,
    pub selection_results: Vec<SelectionResult>,
    pub corrections: Vec<HumanCorrection>,
}
