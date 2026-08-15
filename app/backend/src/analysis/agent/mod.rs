//! Stateless structured model calls for prebuilt analysis prompts.

mod agent;
mod error;

#[cfg(all(test, feature = "paid-openai-test"))]
pub(super) use agent::OpenAiAgent;
pub(super) use agent::{Agent, AnalysisResponse};
#[cfg(test)]
pub(super) use agent::{ChecklistProgress, Observation};
pub(super) use error::Error;
