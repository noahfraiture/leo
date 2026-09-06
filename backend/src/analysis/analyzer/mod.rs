//! Resumable batch execution and durable progress checkpoints.

mod batch;
mod engine;
mod progress;
#[cfg(feature = "test-support")]
mod summary;

pub use crate::analysis::video::AnalysisWarning;
pub use engine::Analyzer;
pub use progress::AnalysisCheckpoint;
#[cfg(feature = "test-support")]
pub use summary::{BatchSummary, PlanSummary};
