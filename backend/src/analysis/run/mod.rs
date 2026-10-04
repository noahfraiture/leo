//! Whole-checklist scanning, one consolidation and at most one independent closer look per criterion.
mod engine;
mod error;
mod jobs;
mod legacy;
mod media;
mod model;
mod recheck;
mod results;
mod review;
mod store;
#[cfg(test)]
mod tests;

pub use engine::{CreateRun, ExecuteRun, create_run, execute_run, request_recheck};
pub use error::{RunError, RunResult};
pub use legacy::ArchivedRun;
pub use model::{
    Consolidation, CriterionAssessment, EvidencePolicy, Finding, HumanCorrection, JobKind,
    JobOutput, JobRecord, JobState, Judgment, OperatorReview, ReviewMark, RunCheckpoint, RunInputs,
    RunSnapshot, RunStage, SavedResponse, ScanFinding, ScanObservation, StudentAssessment,
    StudentOutcome,
};
pub use store::{RunHistory, RunStore};
