use super::super::{EvidenceManifest, EvidenceWindow, SentImage};
use crate::{
    exercise::ExerciseRevision,
    model::{Completion, ExecutionPolicy, ModelSnapshot},
    profiles::{ImageDetailPolicy, ImageSizePolicy},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sampling and opt-in additions, frozen for the lifetime of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidencePolicy {
    pub max_window_span_ms: u64,
    pub overlap_frame_sets: usize,
    pub image_size: ImageSizePolicy,
    pub image_detail: ImageDetailPolicy,
    pub reference_images: bool,
    pub automatic_recheck: bool,
}
impl Default for EvidencePolicy {
    fn default() -> Self {
        Self {
            max_window_span_ms: 7000,
            overlap_frame_sets: 1,
            image_size: ImageSizePolicy::MaximumLongEdge(1280),
            image_detail: ImageDetailPolicy::ProviderDefault,
            reference_images: false,
            automatic_recheck: false,
        }
    }
}

/// Immutable operator configuration, recording identities and one selected model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunInputs {
    pub schema_version: u8,
    pub id: Uuid,
    pub created_utc_ms: u64,
    pub prompt_version: String,
    pub exercise: ExerciseRevision,
    pub model: ModelSnapshot,
    pub execution: ExecutionPolicy,
    pub evidence_policy: EvidencePolicy,
    pub initial_evidence: EvidenceManifest,
    pub windows: Vec<EvidenceWindow>,
}

/// Processing status, separate from a student's results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStage {
    Preparing,
    Scanning,
    Consolidating,
    Rechecking,
    Ready,
    Paused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Scan,
    Consolidate,
    Recheck,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Pending,
    Running,
    Complete,
    Failed { message: String },
}

/// A saved request identity. Rechecks have one unique job per criterion, including retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobRecord {
    pub id: String,
    pub kind: JobKind,
    pub criterion_id: Option<Uuid>,
    pub frame_ids: Vec<String>,
    pub state: JobState,
    /// Processing failures remain visible after explicit resume succeeds.
    pub failures: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunCheckpoint {
    pub stage: RunStage,
    pub jobs: Vec<JobRecord>,
    pub evidence: EvidenceManifest,
    pub assessments: Vec<CriterionAssessment>,
    pub error: Option<String>,
}

/// A candidate judgment. Unknown policy and processing errors are never candidate failures.
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

/// One reported visible fact, associated with an operator criterion and original candidate images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanObservation {
    pub criterion_id: String,
    pub description: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanFinding {
    pub observations: Vec<ScanObservation>,
}

/// Initial consolidation or independent closer look. A configuration issue keeps the result unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Finding {
    pub criterion_id: String,
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub configuration_issue: Option<String>,
    pub conflicting_evidence: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consolidation {
    pub findings: Vec<Finding>,
}

/// Review projection retaining both model decisions; disagreements cannot silently overwrite a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CriterionAssessment {
    pub criterion_id: Uuid,
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub configuration_issue: Option<String>,
    pub initial: Finding,
    pub recheck: Option<Finding>,
    pub recheck_note: Option<String>,
}

/// Persisted response and the exact image identities used for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedResponse {
    pub job_id: String,
    pub frame_ids: Vec<String>,
    pub sent_images: Vec<SentImage>,
    pub reference_hashes: Vec<(Uuid, String)>,
    pub output: JobOutput,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobOutput {
    Scan(Completion<ScanFinding>),
    Consolidate(Completion<Consolidation>),
    Recheck(Completion<Finding>),
}

/// Operator correction history never replaces the saved model response.
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

/// Durable operator review marks and overall assessments for this recording and run only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorReview {
    pub marks: Vec<ReviewMark>,
    pub assessments: Vec<StudentAssessment>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewMark {
    pub criterion_id: Uuid,
    pub basis: String,
    pub created_utc_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StudentOutcome {
    Pass,
    NeedsPractice,
    NeedsEvidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StudentAssessment {
    pub outcome: StudentOutcome,
    pub note: String,
    pub basis: String,
    pub created_utc_ms: u64,
}

/// Complete saved state consumed by the UI, with initial findings, closer looks and human history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSnapshot {
    pub inputs: RunInputs,
    pub checkpoint: RunCheckpoint,
    pub responses: Vec<SavedResponse>,
    pub corrections: Vec<HumanCorrection>,
    pub operator_review: OperatorReview,
}
