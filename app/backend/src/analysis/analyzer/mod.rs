//! Resumable batch execution and durable progress checkpoints.

mod engine;
mod progress;

pub use crate::analysis::video::AnalysisWarning;
pub use engine::Analyzer;
pub use progress::{ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint, AnalysisIdentity};
