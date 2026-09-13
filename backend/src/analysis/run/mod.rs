//! Immutable analysis runs, durable independent jobs, and operator correction history.
mod engine;
mod error;
mod jobs;
mod media;
mod model;
mod selection;
mod selection_aliases;
mod selection_wire;
mod selector_v2;
mod store;
mod workflow;

#[cfg(feature = "test-support")]
mod diagnostic;
#[cfg(feature = "test-support")]
pub use diagnostic::{SelectorDiagnostic, diagnose_selector};

#[cfg(test)]
mod tests;

pub use engine::{CreateRun, ExecuteRun, create_run, execute_run};
pub use error::{RunError, RunResult};
pub use model::{CriterionWindows, FinalResult, ReaderResult, ReviewResult, SelectionResult};
pub use model::{
    EvidencePolicy, HumanCorrection, JobKind, JobRecord, JobState, RunCheckpoint, RunInputs,
    RunSnapshot, RunStage,
};
pub use store::RunStore;
