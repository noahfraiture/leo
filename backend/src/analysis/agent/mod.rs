//! Stateless structured model calls for prebuilt analysis prompts.

mod client;
mod error;
#[cfg(feature = "test-support")]
mod metering;

#[cfg(feature = "test-support")]
pub use client::LocalChatAgent;
pub use client::{
    Agent, AnalysisResponse, ChecklistProgress, Observation, OpenAiAgent, OpenAiConfig,
};
pub use error::Error;
