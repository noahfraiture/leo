//! Stateless structured model calls for prebuilt analysis prompts.

mod agent;
mod error;

pub(super) use agent::{Agent, AnalysisResponse};
#[cfg(test)]
pub(super) use agent::{ChecklistProgress, Observation, OpenAiAgent};
pub(super) use error::Error;
