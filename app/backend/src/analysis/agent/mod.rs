//! Stateless structured model calls for prebuilt analysis prompts.

mod client;
mod error;

pub use client::{Agent, AnalysisResponse, ChecklistProgress, Observation, OpenAiAgent};
pub use error::Error;
