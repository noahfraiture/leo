//! Stateless structured model calls for prebuilt analysis prompts.

mod client;
mod error;

pub use client::OpenAiConfiguration;
pub use client::{Agent, AnalysisResponse, ChecklistProgress, Observation};
pub use error::Error;
