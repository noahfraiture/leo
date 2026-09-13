//! Immutable analysis runs, durable independent jobs, and operator correction history.
mod engine;
mod error;
mod jobs;
mod media;
mod model;
mod selection;
mod store;
mod workflow;

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
