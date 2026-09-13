//! Immutable analysis runs, durable independent jobs, and operator correction history.
mod error;
mod jobs;
mod media;
mod model;
mod selection;
mod store;
mod workflow;


pub use error::{RunError, RunResult};
pub use model::{CriterionWindows, FinalResult, ReaderResult, ReviewResult, SelectionResult};
pub use model::{
    EvidencePolicy, HumanCorrection, JobKind, JobRecord, JobState, RunCheckpoint, RunInputs,
    RunSnapshot, RunStage,
};
pub use store::RunStore;
