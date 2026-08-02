//! Resumable batch execution and durable progress checkpoints.

mod error;
mod progress;
mod runner;

pub(crate) use error::Error;
pub(crate) use runner::AnalysisRunner;
