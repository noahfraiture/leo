//! Resumable batch execution and durable progress checkpoints.

mod batch;
mod engine;
mod progress;

pub use crate::analysis::video::AnalysisWarning;
pub use engine::Analyzer;
pub use progress::AnalysisCheckpoint;
